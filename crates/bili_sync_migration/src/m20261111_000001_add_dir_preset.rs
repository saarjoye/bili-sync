use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DirPreset::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DirPreset::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(DirPreset::Alias).string_len(64).not_null())
                    // 本地下载根目录（相对 bili-sync 的下载根，或绝对路径）
                    .col(ColumnDef::new(DirPreset::LocalPath).string_len(512).not_null())
                    // 对应的 115 目标路径（仅作映射记录，bili-sync 不使用）
                    .col(
                        ColumnDef::new(DirPreset::Target115Path)
                            .string_len(512)
                            .not_null()
                            .default(""),
                    )
                    // 默认季号：0 = 不生成季目录
                    .col(
                        ColumnDef::new(DirPreset::DefaultSeason)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    // 作者目录显示方式：0=UP名 1=UP名-ID 2=仅ID
                    .col(
                        ColumnDef::new(DirPreset::UpperDisplay)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(
                        ColumnDef::new(DirPreset::SortOrder)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(DirPreset::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DirPreset::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum DirPreset {
    Table,
    Id,
    Alias,
    LocalPath,
    Target115Path,
    DefaultSeason,
    UpperDisplay,
    SortOrder,
    CreatedAt,
}
