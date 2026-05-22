//! Database migrations for the Analytics API service.

mod m20260414_000001_init;
mod m20260422_000001_seed_metrics;
mod m20260423_000001_seed_metrics_honest_nulls;
mod m20260428_000001_collab_metrics_update;
mod m20260429_000001_task_delivery_silver_rewrite;
mod m20260430_000001_update_git_bullet;
mod m20260507_000001_seed_crm_metrics;
mod m20260515_000001_task_delivery_bullet_rewrite;
mod m20260518_000001_collab_bullet_rewrite;
mod m20260519_000001_ai_bullet_rewrite;
mod m20260520_000001_code_quality_bullet_rewrite;

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260414_000001_init::Migration),
            Box::new(m20260422_000001_seed_metrics::Migration),
            Box::new(m20260423_000001_seed_metrics_honest_nulls::Migration),
            Box::new(m20260428_000001_collab_metrics_update::Migration),
            Box::new(m20260429_000001_task_delivery_silver_rewrite::Migration),
            Box::new(m20260430_000001_update_git_bullet::Migration),
            Box::new(m20260507_000001_seed_crm_metrics::Migration),
            Box::new(m20260515_000001_task_delivery_bullet_rewrite::Migration),
            Box::new(m20260518_000001_collab_bullet_rewrite::Migration),
            Box::new(m20260519_000001_ai_bullet_rewrite::Migration),
            Box::new(m20260520_000001_code_quality_bullet_rewrite::Migration),
        ]
    }
}
