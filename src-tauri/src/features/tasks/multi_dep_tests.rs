use crate::db::init_db;
use crate::features::tasks::{CreateTaskDto, TaskService};

#[tokio::test]
async fn test_multi_dependencies_and_prompt_chaining() {
    let dir = std::env::temp_dir().join(format!("agent_board_dep_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let pool = init_db(db_path.to_str().unwrap()).await.unwrap();

    // 1. 建立前置任務 A 與 B
    let task_a = TaskService::create_task(
        &pool,
        CreateTaskDto {
            title: "Task A".into(),
            prompt: "Do A".into(),
            creator: Some("human".into()),
            assigned_agent: Some("agy".into()),
            dependency_ids: None,
            model: None,
            workdir: None,
            auto_approve: Some(true),
            isolate: Some(false),
        },
    )
    .await
    .unwrap();

    let task_b = TaskService::create_task(
        &pool,
        CreateTaskDto {
            title: "Task B".into(),
            prompt: "Do B".into(),
            creator: Some("human".into()),
            assigned_agent: Some("agy".into()),
            dependency_ids: None,
            model: None,
            workdir: None,
            auto_approve: Some(true),
            isolate: Some(false),
        },
    )
    .await
    .unwrap();

    // 2. 建立下游任務 C，同時依賴 A 與 B
    let task_c = TaskService::create_task(
        &pool,
        CreateTaskDto {
            title: "Task C".into(),
            prompt: "綜合結果：\n{{dep.result}}".into(),
            creator: Some("human".into()),
            assigned_agent: Some("agy".into()),
            dependency_ids: Some(vec![task_a.id.clone(), task_b.id.clone()]),
            model: None,
            workdir: None,
            auto_approve: Some(true),
            isolate: Some(false),
        },
    )
    .await
    .unwrap();

    // 3. 此時 A 與 B 均為 queued，Claim C 應該拿不到（因為前置未完成）
    let claimed_c_early = TaskService::claim_specific(
        &pool,
        &task_c.id,
        "agy",
        &Default::default(),
    )
    .await
    .unwrap();
    assert!(claimed_c_early.is_none(), "前置任務未完成前不應被領取");

    // 4. 完成任務 A（回報成果），此時 B 仍未完成，C 仍不可領取
    let claimed_a = TaskService::claim_specific(&pool, &task_a.id, "agy", &Default::default())
        .await
        .unwrap()
        .unwrap();
    TaskService::set_result(&pool, &task_a.id, &crate::features::agents::stream::RunResult {
        text: "A 完成產出: 報告A".into(),
        ..Default::default()
    }).await.unwrap();
    TaskService::complete_task(&pool, &task_a.id, &claimed_a.lease_id)
        .await
        .unwrap();

    let claimed_c_mid = TaskService::claim_specific(&pool, &task_c.id, "agy", &Default::default())
        .await
        .unwrap();
    assert!(claimed_c_mid.is_none(), "僅完成部分前置時依然不能領取");

    // 5. 完成任務 B（回報成果）
    let claimed_b = TaskService::claim_specific(&pool, &task_b.id, "agy", &Default::default())
        .await
        .unwrap()
        .unwrap();
    TaskService::set_result(&pool, &task_b.id, &crate::features::agents::stream::RunResult {
        text: "B 完成產出: 模組B".into(),
        ..Default::default()
    }).await.unwrap();
    TaskService::complete_task(&pool, &task_b.id, &claimed_b.lease_id)
        .await
        .unwrap();

    // 6. 此時 A 與 B 均已 completed，C 應該可以成功被領取
    let claimed_c = TaskService::claim_specific(&pool, &task_c.id, "agy", &Default::default())
        .await
        .unwrap()
        .expect("A 與 B 皆完成後，C 應順利被領取");

    // 7. 驗證 command (argv) 是否同時串接了兩者的結果，而 task.prompt 保留範本
    let executed_cmd = claimed_c.command.unwrap().join(" ");
    assert!(executed_cmd.contains("報告A"));
    assert!(executed_cmd.contains("模組B"));
    assert!(claimed_c.task.prompt.contains("{{dep.result}}"));

    let _ = std::fs::remove_dir_all(&dir);
}
