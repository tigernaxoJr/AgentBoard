use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Artifact {
    pub id: String,
    pub task_id: String,
    pub agent_id: String,
    pub file_path: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateArtifactDto {
    pub agent_id: String,
    pub file_path: String,
    pub summary: String,
}

pub struct ArtifactService;

impl ArtifactService {
    pub async fn create_artifact(
        pool: &Pool<Sqlite>,
        task_id: &str,
        dto: CreateArtifactDto,
    ) -> Result<Artifact, sqlx::Error> {
        let artifact = Artifact {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            agent_id: dto.agent_id,
            file_path: dto.file_path,
            summary: dto.summary,
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query(
            r#"
            INSERT INTO artifacts (id, task_id, agent_id, file_path, summary, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&artifact.id)
        .bind(&artifact.task_id)
        .bind(&artifact.agent_id)
        .bind(&artifact.file_path)
        .bind(&artifact.summary)
        .bind(&artifact.created_at)
        .execute(pool)
        .await?;

        Ok(artifact)
    }

    pub async fn get(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Artifact>, sqlx::Error> {
        sqlx::query_as::<_, Artifact>("SELECT * FROM artifacts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_by_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
    ) -> Result<Vec<Artifact>, sqlx::Error> {
        let items = sqlx::query_as::<_, Artifact>(
            "SELECT * FROM artifacts WHERE task_id = ? ORDER BY created_at DESC",
        )
        .bind(task_id)
        .fetch_all(pool)
        .await?;
        Ok(items)
    }
}

/// 任務的輸出資料夾（相對於執行目錄）。prompt 只說「檔案存到這裡」，Agent 不需要知道任務 id 或平台存在
pub fn output_dir(task_id: &str) -> String {
    format!("outputs/{}", opaque_name(task_id))
}

/// 由任務 id 推出的目錄名稱（FNV-1a 雜湊，固定演算法所以重啟後仍對得上）。
/// 不直接用 id 前綴，避免 Agent 拿目錄名去對應任務
pub fn opaque_name(task_id: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in task_id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", h as u32)
}

/// 任務完成後，把輸出資料夾內的檔案登記為成果（已登記過的路徑不重複登記）
pub async fn register_outputs(
    pool: &Pool<Sqlite>,
    task_id: &str,
    agent_id: &str,
    roots: &[&str],
) -> Result<(), sqlx::Error> {
    let rel = output_dir(task_id);
    let existing: Vec<String> = ArtifactService::list_by_task(pool, task_id)
        .await?
        .into_iter()
        .map(|a| a.file_path)
        .collect();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(std::path::Path::new(root).join(&rel)) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let file_path = format!("{rel}/{}", entry.file_name().to_string_lossy());
            if existing.contains(&file_path) {
                continue;
            }
            let dto = CreateArtifactDto {
                agent_id: agent_id.to_string(),
                summary: entry.file_name().to_string_lossy().into_owned(),
                file_path,
            };
            ArtifactService::create_artifact(pool, task_id, dto).await?;
        }
    }
    Ok(())
}

const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// 成果檔案的內容類型；只回傳瀏覽器可直接顯示的圖片，其餘一律當下載
pub fn content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// 解析成果檔案的實際路徑。路徑由 Agent 回報，必須落在任務的工作目錄或 worktree 內
/// （正規化後比對，擋掉 `..` 與符號連結逃逸），且大小有上限
pub fn resolve_file(
    file_path: &str,
    workdir: Option<&str>,
    worktree: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    let roots: Vec<_> = [worktree, workdir].into_iter().flatten().collect();
    let raw = std::path::Path::new(file_path);
    for root in roots {
        let candidate = if raw.is_absolute() { raw.to_path_buf() } else { std::path::Path::new(root).join(raw) };
        let (Ok(real), Ok(base)) = (candidate.canonicalize(), std::path::Path::new(root).canonicalize()) else {
            continue;
        };
        if real.starts_with(&base) && real.is_file() {
            let len = std::fs::metadata(&real).map_err(|e| e.to_string())?.len();
            if len > MAX_FILE_BYTES {
                return Err("檔案超過 20MB".into());
            }
            return Ok(real);
        }
    }
    Err("找不到檔案，或檔案不在任務的工作目錄內".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_file_stays_inside_workdir() {
        let dir = std::env::temp_dir().join(format!("ab-art-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("out/a.png"), b"x").unwrap();
        let root = dir.to_str().unwrap();
        assert!(resolve_file("out/a.png", Some(root), None).is_ok());
        assert!(resolve_file(dir.join("out/a.png").to_str().unwrap(), Some(root), None).is_ok());
        assert!(resolve_file("../../etc/passwd", Some(root), None).is_err());
        assert!(resolve_file("/etc/passwd", Some(root), None).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
