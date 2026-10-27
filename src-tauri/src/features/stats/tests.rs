use crate::db::init_db;
use crate::features::stats::{StatsQuery, StatsService};
use crate::features::tasks::{CreateTaskDto, TaskService};

#[tokio::test]
async fn test_stats_and_templates_lifecycle() {
    let dir = std::env::temp_dir().join(format!("agent_board_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let pool = init_db(db_path.to_str().unwrap()).await.unwrap();

    // 1. Create a task and test initial stats
    let _task = TaskService::create_task(
        &pool,
        CreateTaskDto {
            title: "測試統計任務".into(),
            prompt: "測試內容".into(),
            creator: Some("test".into()),
            assigned_agent: Some("claude".into()),
            depends_on_task_id: None,
            dependency_ids: None,
            model: Some("sonnet".into()),
            workdir: Some("/test/dir".into()),
            auto_approve: Some(true),
            isolate: Some(false),
        },
    )
    .await
    .unwrap();

    let stats = StatsService::get_stats(&pool, StatsQuery::default()).await.unwrap();
    assert_eq!(stats.summary.total_tasks, 1);
    assert_eq!(stats.summary.queued_tasks, 1);
    assert_eq!(stats.by_agent.len(), 1);
    assert_eq!(stats.by_agent[0].agent, "claude");

    // 2. Test template CRUD
    use crate::features::templates::{CreateTemplateDto, TemplateService, UpdateTemplateDto};

    let tmpl = TemplateService::create(
        &pool,
        CreateTemplateDto {
            name: "標準重構範本".into(),
            description: Some("重構程式碼並加單元測試".into()),
            title: "重構功能".into(),
            prompt: "請針對 {{file}} 重構".into(),
            assigned_agent: Some("agy".into()),
            model: Some("flash".into()),
            workdir: Some("/app".into()),
            auto_approve: Some(true),
            isolate: Some(true),
            priority: Some(5),
        },
    )
    .await
    .unwrap();

    assert_eq!(tmpl.name, "標準重構範本");

    let list = TemplateService::list(&pool).await.unwrap();
    assert_eq!(list.len(), 1);

    let updated = TemplateService::update(
        &pool,
        &tmpl.id,
        UpdateTemplateDto {
            name: Some("已更新的範本".into()),
            description: None,
            title: None,
            prompt: None,
            assigned_agent: None,
            model: None,
            workdir: None,
            auto_approve: None,
            isolate: None,
            priority: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(updated.name, "已更新的範本");

    let deleted = TemplateService::delete(&pool, &tmpl.id).await.unwrap();
    assert!(deleted);

    let list_after = TemplateService::list(&pool).await.unwrap();
    assert_eq!(list_after.len(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn empty_db_stats_do_not_error_and_to_date_is_inclusive() {
    let dir = std::env::temp_dir().join(format!("agent_board_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(dir.join("t.db").to_str().unwrap()).await.unwrap();

    let empty = StatsService::get_stats(&pool, StatsQuery::default()).await.unwrap();
    assert_eq!(empty.summary.total_tasks, 0);
    assert_eq!(empty.summary.completed_tasks, 0);

    TaskService::create_task(&pool, CreateTaskDto { title: "t".into(), prompt: "p".into(), ..Default::default() })
        .await
        .unwrap();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let q = StatsQuery { from: Some(today.clone()), to: Some(today) };
    assert_eq!(StatsService::get_stats(&pool, q).await.unwrap().summary.total_tasks, 1);

    let _ = std::fs::remove_dir_all(&dir);
}
