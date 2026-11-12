use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// 注意：SQLite 不支持一条 ALTER TABLE 里加多列（sea-query 会 panic），
/// 所以这里每列单独一条语句。
const TABLES: [&str; 4] = ["collection", "favorite", "submission", "watch_later"];

macro_rules! add_cols {
    ($manager:expr, $table:expr) => {{
        let m = $manager;
        let t = $table;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(
                    ColumnDef::new(Alias::new("schedule_mode"))
                        .string_len(16)
                        .not_null()
                        .default("inherit"),
                )
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(
                    ColumnDef::new(Alias::new("interval_seconds"))
                        .integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(
                    ColumnDef::new(Alias::new("cron_expr"))
                        .string_len(64)
                        .not_null()
                        .default(""),
                )
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(ColumnDef::new(Alias::new("next_run_at")).timestamp().null())
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(ColumnDef::new(Alias::new("last_checked_at")).timestamp().null())
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(
                    ColumnDef::new(Alias::new("last_found_count"))
                        .integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(ColumnDef::new(Alias::new("fail_count")).integer().not_null().default(0))
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(ColumnDef::new(Alias::new("max_days")).integer().not_null().default(0))
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Alias::new(t))
                .add_column(ColumnDef::new(Alias::new("max_pages")).integer().not_null().default(0))
                .to_owned(),
        )
        .await?;
    }};
}

macro_rules! drop_cols {
    ($manager:expr, $table:expr) => {{
        let m = $manager;
        let t = $table;
        for col in [
            "max_pages",
            "max_days",
            "fail_count",
            "last_found_count",
            "last_checked_at",
            "next_run_at",
            "cron_expr",
            "interval_seconds",
            "schedule_mode",
        ] {
            m.alter_table(
                Table::alter()
                    .table(Alias::new(t))
                    .drop_column(Alias::new(col))
                    .to_owned(),
            )
            .await?;
        }
    }};
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            add_cols!(manager, table);
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            drop_cols!(manager, table);
        }
        Ok(())
    }
}
