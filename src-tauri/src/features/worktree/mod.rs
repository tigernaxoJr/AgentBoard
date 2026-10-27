use crate::features::tasks::{ClaimedTask, Task, TaskService};
use serde::Serialize;
use sqlx::{Pool, Sqlite};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// 隔離執行：每個任務在 `<root>/worktrees/<任務 ID 前 8 碼>` 的獨立 worktree 與
/// `agent-board/<任務 ID 前 8 碼>` 分支上執行，多個 Agent 同時改同一個 repo 也不會互相覆蓋。
/// worktree 不放在 repo 的 .git 底下，因為部分 Agent CLI 會拒絕寫入 .git 內的路徑。
#[derive(Debug, Serialize)]
pub struct WorktreeStatus {
    pub branch: String,
    pub path: String,
    pub base_commit: String,
    pub exists: bool,
    /// 分支上相對起點的新 commit（`git log --oneline`）
    pub commits: Vec<String>,
    /// 尚未 commit 的變更（`git status --porcelain`）
    pub changes: Vec<String>,
    /// 相對起點的差異統計，含尚未 commit 的變更
    pub diff_stat: String,
}

async fn git(dir: impl AsRef<Path>, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("無法執行 git：{e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(format!(
            "git {} 失敗：{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn lines(s: String) -> Vec<String> {
    s.lines()
        .filter(|l| !l.is_empty())
        .map(Into::into)
        .collect()
}

async fn repo_root(task: &Task) -> Result<PathBuf, String> {
    let workdir = task
        .workdir
        .as_deref()
        .ok_or("隔離執行需要指定工作目錄（git repo 內的路徑）")?;
    git(workdir, &["rev-parse", "--show-toplevel"])
        .await
        .map(PathBuf::from)
        .map_err(|_| format!("{workdir} 不在 git repo 內，無法隔離執行"))
}

/// 領取後呼叫：需要隔離就建立（或沿用上次的）worktree，並把 run_dir 指到 worktree 內對應的目錄
pub async fn prepare(
    pool: &Pool<Sqlite>,
    claimed: &mut ClaimedTask,
    root: &Path,
) -> Result<(), String> {
    if !claimed.task.isolate {
        return Ok(());
    }
    let task = &claimed.task;
    let repo = repo_root(task).await?;
    let workdir = Path::new(task.workdir.as_deref().unwrap_or_default());
    // workdir 可能是 repo 的子目錄，在 worktree 中要對應到同一個子目錄
    let relative = workdir
        .canonicalize()
        .ok()
        .and_then(|w| {
            repo.canonicalize()
                .ok()
                .and_then(|r| w.strip_prefix(r).ok().map(Path::to_path_buf))
        })
        .unwrap_or_default();

    let short = &task.id[..8];
    let branch = task
        .branch
        .clone()
        .unwrap_or_else(|| format!("agent-board/{short}"));
    let path = task
        .worktree_path
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("worktrees").join(short));

    let base = if path.exists() {
        // 重試時沿用上次的 worktree，接續做到一半的內容
        task.base_commit.clone().unwrap_or_default()
    } else {
        let base = match &task.base_commit {
            Some(base) => base.clone(),
            None => git(&repo, &["rev-parse", "HEAD"]).await?,
        };
        std::fs::create_dir_all(root.join("worktrees"))
            .map_err(|e| format!("無法建立 worktree 目錄：{e}"))?;
        let path_str = path.to_string_lossy();
        let branch_exists = git(
            &repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .await
        .is_ok();
        if branch_exists {
            git(&repo, &["worktree", "add", &path_str, &branch]).await?;
        } else {
            git(&repo, &["worktree", "add", "-b", &branch, &path_str, &base]).await?;
        }
        base
    };

    let path_str = path.to_string_lossy().into_owned();
    TaskService::set_worktree(pool, &task.id, Some((&path_str, &branch, &base)))
        .await
        .map_err(|e| e.to_string())?;
    claimed.task.worktree_path = Some(path_str);
    claimed.task.branch = Some(branch);
    claimed.task.base_commit = Some(base);
    claimed.run_dir = Some(path.join(relative).to_string_lossy().into_owned());
    Ok(())
}

fn worktree_of(task: &Task) -> Result<(String, String, String), String> {
    match (&task.worktree_path, &task.branch, &task.base_commit) {
        (Some(p), Some(b), Some(c)) => Ok((p.clone(), b.clone(), c.clone())),
        _ => Err("這個任務沒有 worktree".into()),
    }
}

pub async fn status(task: &Task) -> Result<WorktreeStatus, String> {
    let (path, branch, base_commit) = worktree_of(task)?;
    let exists = Path::new(&path).exists();
    let (commits, changes, diff_stat) = if exists {
        let range = format!("{base_commit}..HEAD");
        (
            lines(git(&path, &["log", "--oneline", &range]).await?),
            lines(git(&path, &["status", "--porcelain"]).await?),
            git(&path, &["diff", "--stat", &base_commit]).await?,
        )
    } else {
        Default::default()
    };
    Ok(WorktreeStatus {
        branch,
        path,
        base_commit,
        exists,
        commits,
        changes,
        diff_stat,
    })
}

/// 合併回 repo 目前的分支：先 commit worktree 內未 commit 的變更，再 `merge --no-ff`
/// 成功後移除 worktree 與分支；失敗（例如衝突）會 abort，repo 維持原狀
pub async fn merge(pool: &Pool<Sqlite>, task: &Task) -> Result<String, String> {
    let (path, branch, base) = worktree_of(task)?;
    let repo = repo_root(task).await?;
    let short = &task.id[..8];

    if Path::new(&path).exists() && !git(&path, &["status", "--porcelain"]).await?.is_empty() {
        git(&path, &["add", "-A"]).await?;
        let message = format!("{}\n\nAgent Board task #{short}", task.title);
        git(&path, &["commit", "-m", &message]).await?;
    }

    let commits = lines(git(&repo, &["log", "--oneline", &format!("{base}..{branch}")]).await?);
    if commits.is_empty() {
        discard(pool, task).await?;
        return Ok("沒有任何變更，已移除 worktree".into());
    }

    let message = format!("Merge agent-board task #{short}: {}", task.title);
    if let Err(e) = git(&repo, &["merge", "--no-ff", &branch, "-m", &message]).await {
        // 衝突時 merge 會停在一半，abort 讓 repo 回到原狀；沒有進行中的 merge 時 abort 失敗可忽略
        let _ = git(&repo, &["merge", "--abort"]).await;
        return Err(e);
    }
    discard(pool, task).await?;
    Ok(format!("已合併 {} 個 commit", commits.len()))
}

/// 取得 worktree 中指定檔案（或全部檔案）相對於 base_commit 的完整 diff 內容
pub async fn diff(task: &Task, file_path: Option<&str>) -> Result<String, String> {
    let (path, _, base_commit) = worktree_of(task)?;
    if !Path::new(&path).exists() {
        return Err("worktree 不存在".into());
    }

    let filter = file_path.filter(|p| !p.trim().is_empty());
    let mut args = vec!["diff", &base_commit];
    if let Some(fp) = filter {
        args.push("--");
        args.push(fp);
    }
    let mut out = git(&path, &args).await?;

    // 未追蹤的新檔案不會出現在 git diff；不動 index（避免 GET 有副作用），直接列出內容
    const MAX_UNTRACKED_BYTES: usize = 200 * 1024;
    let mut untracked = vec!["ls-files", "--others", "--exclude-standard"];
    if let Some(fp) = filter {
        untracked.push("--");
        untracked.push(fp);
    }
    for rel in git(&path, &untracked)
        .await?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let Ok(bytes) = std::fs::read(Path::new(&path).join(rel)) else {
            continue;
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("--- /dev/null\n+++ b/{rel}  (新檔案)\n"));
        let head = &bytes[..bytes.len().min(MAX_UNTRACKED_BYTES)];
        if head.contains(&0) {
            out.push_str("（二進位檔案，略過內容）\n");
            continue;
        }
        for line in String::from_utf8_lossy(head).lines() {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// 捨棄：移除 worktree 與分支（其中的變更會一併刪除）
pub async fn discard(pool: &Pool<Sqlite>, task: &Task) -> Result<(), String> {
    let (path, branch, _) = worktree_of(task)?;
    let repo = repo_root(task).await?;
    if Path::new(&path).exists() {
        git(&repo, &["worktree", "remove", "--force", &path]).await?;
    } else {
        git(&repo, &["worktree", "prune"]).await?;
    }
    let _ = git(&repo, &["branch", "-D", &branch]).await;
    TaskService::set_worktree(pool, &task.id, None)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::tasks::{CreateTaskDto, RunOptions};
    use sqlx::sqlite::SqlitePoolOptions;

    struct Fixture {
        pool: Pool<Sqlite>,
        dir: PathBuf,
        repo: PathBuf,
        root: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn fixture() -> Fixture {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrate(&pool).await.unwrap();
        let dir = std::env::temp_dir().join(format!("agent_board_wt_{}", uuid::Uuid::new_v4()));
        let (repo, root) = (dir.join("repo"), dir.join("data"));
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join("sub/b.txt"), "b\n").unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"],
            &["config", "user.name", "Test"],
            &["add", "-A"],
            &["commit", "-qm", "init"],
        ] {
            git(&repo, args).await.unwrap();
        }
        Fixture {
            pool,
            dir,
            repo,
            root,
        }
    }

    async fn claim_isolated(f: &Fixture, workdir: &Path) -> ClaimedTask {
        let dto = CreateTaskDto {
            title: "改 a.txt".into(),
            prompt: "p".into(),
            ..Default::default()
        };
        let task = TaskService::create_task(&f.pool, dto).await.unwrap();
        let opts = RunOptions {
            workdir: Some(workdir.to_string_lossy().into()),
            isolate: Some(true),
            ..Default::default()
        };
        let mut claimed = TaskService::claim_specific(&f.pool, &task.id, "agy", &opts)
            .await
            .unwrap()
            .unwrap();
        prepare(&f.pool, &mut claimed, &f.root).await.unwrap();
        claimed
    }

    async fn reload(f: &Fixture, id: &str) -> Task {
        TaskService::get_task(&f.pool, id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn prepare_creates_worktree_and_maps_subdirectory() {
        let f = fixture().await;
        let claimed = claim_isolated(&f, &f.repo.join("sub")).await;
        let wt = PathBuf::from(claimed.task.worktree_path.clone().unwrap());

        assert!(wt.starts_with(f.root.join("worktrees")));
        assert_eq!(PathBuf::from(claimed.run_dir.unwrap()), wt.join("sub"));
        assert!(wt.join("sub/b.txt").exists());
        let saved = reload(&f, &claimed.task.id).await;
        assert_eq!(
            saved.branch.as_deref(),
            Some(format!("agent-board/{}", &claimed.task.id[..8]).as_str())
        );
    }

    #[tokio::test]
    async fn retry_reuses_existing_worktree() {
        let f = fixture().await;
        let mut claimed = claim_isolated(&f, &f.repo).await;
        let wt = PathBuf::from(claimed.task.worktree_path.clone().unwrap());
        std::fs::write(wt.join("partial.txt"), "half done\n").unwrap();

        prepare(&f.pool, &mut claimed, &f.root).await.unwrap();
        assert!(wt.join("partial.txt").exists());
    }

    #[tokio::test]
    async fn merge_commits_pending_changes_and_cleans_up() {
        let f = fixture().await;
        let claimed = claim_isolated(&f, &f.repo).await;
        let wt = PathBuf::from(claimed.task.worktree_path.clone().unwrap());
        std::fs::write(wt.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();

        let task = reload(&f, &claimed.task.id).await;
        let st = status(&task).await.unwrap();
        assert!(st.exists && st.commits.is_empty());
        assert_eq!(st.changes.len(), 2);
        assert!(st.diff_stat.contains("a.txt"));

        let msg = merge(&f.pool, &task).await.unwrap();
        assert_eq!(msg, "已合併 1 個 commit");
        assert_eq!(
            std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
            "one\ntwo\n"
        );
        assert!(f.repo.join("new.txt").exists());
        assert!(!wt.exists());
        assert!(
            git(
                &f.repo,
                &["rev-parse", "--verify", "--quiet", &task.branch.unwrap()]
            )
            .await
            .is_err()
        );
        assert!(reload(&f, &claimed.task.id).await.worktree_path.is_none());
    }

    #[tokio::test]
    async fn conflicting_merge_is_aborted_and_keeps_worktree() {
        let f = fixture().await;
        let claimed = claim_isolated(&f, &f.repo).await;
        let wt = PathBuf::from(claimed.task.worktree_path.clone().unwrap());
        std::fs::write(wt.join("a.txt"), "agent version\n").unwrap();
        // 主分支同時改了同一行
        std::fs::write(f.repo.join("a.txt"), "human version\n").unwrap();
        git(&f.repo, &["commit", "-qam", "human edit"])
            .await
            .unwrap();

        let task = reload(&f, &claimed.task.id).await;
        assert!(merge(&f.pool, &task).await.is_err());
        assert_eq!(
            std::fs::read_to_string(f.repo.join("a.txt")).unwrap(),
            "human version\n"
        );
        assert!(
            git(&f.repo, &["status", "--porcelain"])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(wt.exists());
        assert!(reload(&f, &claimed.task.id).await.worktree_path.is_some());
    }

    #[tokio::test]
    async fn discard_removes_worktree_and_branch() {
        let f = fixture().await;
        let claimed = claim_isolated(&f, &f.repo).await;
        let task = reload(&f, &claimed.task.id).await;
        discard(&f.pool, &task).await.unwrap();

        assert!(!Path::new(&task.worktree_path.unwrap()).exists());
        assert!(
            git(&f.repo, &["branch", "--list", &task.branch.unwrap()])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn isolation_requires_a_git_repo() {
        let f = fixture().await;
        let outside = f.dir.join("not-a-repo");
        std::fs::create_dir_all(&outside).unwrap();
        let dto = CreateTaskDto {
            title: "t".into(),
            prompt: "p".into(),
            ..Default::default()
        };
        let task = TaskService::create_task(&f.pool, dto).await.unwrap();
        let opts = RunOptions {
            workdir: Some(outside.to_string_lossy().into()),
            isolate: Some(true),
            ..Default::default()
        };
        let mut claimed = TaskService::claim_specific(&f.pool, &task.id, "agy", &opts)
            .await
            .unwrap()
            .unwrap();
        let err = prepare(&f.pool, &mut claimed, &f.root).await.unwrap_err();
        assert!(err.contains("不在 git repo 內"), "{err}");
    }
}
