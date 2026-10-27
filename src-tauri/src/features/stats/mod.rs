use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StatsSummary {
    pub total_tasks: i64,
    pub completed_tasks: i64,
    pub failed_tasks: i64,
    pub running_tasks: i64,
    pub queued_tasks: i64,
    pub total_cost_usd: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentStat {
    pub agent: String,
    pub task_count: i64,
    pub completed_count: i64,
    pub failed_count: i64,
    pub total_cost_usd: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkdirStat {
    pub workdir: String,
    pub task_count: i64,
    pub total_cost_usd: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DailyStat {
    pub date: String,
    pub task_count: i64,
    pub total_cost_usd: f64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullStatsResponse {
    pub summary: StatsSummary,
    pub by_agent: Vec<AgentStat>,
    pub by_workdir: Vec<WorkdirStat>,
    pub by_date: Vec<DailyStat>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

pub struct StatsService;

impl StatsService {
    pub async fn get_stats(pool: &Pool<Sqlite>, query: StatsQuery) -> Result<FullStatsResponse, sqlx::Error> {
        let from_date = query.from.unwrap_or_else(|| "1970-01-01".to_string());
        // 純日期（YYYY-MM-DD）視為包含當天整天，否則字串比較會漏掉當天任務
        let to_date = match query.to {
            Some(t) if t.len() == 10 => format!("{t}T23:59:59.999999999~"),
            Some(t) => t,
            None => "2099-12-31T23:59:59".to_string(),
        };

        // 1. Overall counts & totals
        let summary = sqlx::query_as::<_, StatsSummary>(
            r#"
            SELECT
                COUNT(*) as total_tasks,
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0) as completed_tasks,
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) as failed_tasks,
                COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0) as running_tasks,
                COALESCE(SUM(CASE WHEN status = 'queued' THEN 1 ELSE 0 END), 0) as queued_tasks,
                COALESCE(SUM(cost_usd), 0.0) as total_cost_usd,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(duration_ms), 0) as total_duration_ms
            FROM tasks
            WHERE created_at >= ? AND created_at <= ?
            "#,
        )
        .bind(&from_date)
        .bind(&to_date)
        .fetch_one(pool)
        .await?;

        // 2. By Agent
        let by_agent = sqlx::query_as::<_, AgentStat>(
            r#"
            SELECT
                COALESCE(assigned_agent, 'unassigned') as agent,
                COUNT(*) as task_count,
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0) as completed_count,
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) as failed_count,
                COALESCE(SUM(cost_usd), 0.0) as total_cost_usd,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(duration_ms), 0) as total_duration_ms
            FROM tasks
            WHERE created_at >= ? AND created_at <= ?
            GROUP BY COALESCE(assigned_agent, 'unassigned')
            ORDER BY task_count DESC
            "#,
        )
        .bind(&from_date)
        .bind(&to_date)
        .fetch_all(pool)
        .await?;

        // 3. By Workdir (top 15)
        let by_workdir = sqlx::query_as::<_, WorkdirStat>(
            r#"
            SELECT
                COALESCE(workdir, '(預設工作目錄)') as workdir,
                COUNT(*) as task_count,
                COALESCE(SUM(cost_usd), 0.0) as total_cost_usd,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(duration_ms), 0) as total_duration_ms
            FROM tasks
            WHERE created_at >= ? AND created_at <= ?
            GROUP BY COALESCE(workdir, '(預設工作目錄)')
            ORDER BY task_count DESC
            LIMIT 15
            "#,
        )
        .bind(&from_date)
        .bind(&to_date)
        .fetch_all(pool)
        .await?;

        // 4. By Date（最近 90 筆，輸出為升冪）
        let by_date = sqlx::query_as::<_, DailyStat>(
            r#"
            SELECT
                SUBSTR(created_at, 1, 10) as date,
                COUNT(*) as task_count,
                COALESCE(SUM(cost_usd), 0.0) as total_cost_usd,
                COALESCE(SUM(input_tokens + output_tokens), 0) as total_tokens
            FROM tasks
            WHERE created_at >= ? AND created_at <= ?
            GROUP BY SUBSTR(created_at, 1, 10)
            ORDER BY date DESC
            LIMIT 90
            "#,
        )
        .bind(&from_date)
        .bind(&to_date)
        .fetch_all(pool)
        .await?;
        // 取最近 90 天後改為由舊到新，圖表可直接使用
        let by_date: Vec<DailyStat> = by_date.into_iter().rev().collect();

        Ok(FullStatsResponse {
            summary,
            by_agent,
            by_workdir,
            by_date,
        })
    }
}

#[cfg(test)]
mod tests;

