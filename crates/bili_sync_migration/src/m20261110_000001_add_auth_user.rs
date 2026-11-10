use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AuthUser::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(AuthUser::Id).integer().not_null().auto_increment().primary_key())
                    .col(ColumnDef::new(AuthUser::Username).string_len(64).not_null().unique_key())
                    .col(ColumnDef::new(AuthUser::PasswordHash).string_len(256).not_null())
                    .col(ColumnDef::new(AuthUser::Salt).string_len(64).not_null())
                    .col(ColumnDef::new(AuthUser::CreatedAt).string_len(32).not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(AuthUser::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum AuthUser {
    Table,
    Id,
    Username,
    PasswordHash,
    Salt,
    CreatedAt,
}
