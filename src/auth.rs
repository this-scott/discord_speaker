use crate::db;
use chrono::{DateTime, Utc};
/// Looks up user against sqllite db to get and apply refresh token. Returns user token or creates one if none found
pub async fn get_user_credential(db_ctx: &db::DBContext, user: &serenity::model::user::User) -> Result<Option<String>, tokio_rusqlite::Error> {
    println!("User details: {:?}", user);
    let user_record = match db_ctx.lookup_user(user.id.get()).await {
        Ok(user_record) => user_record,
        // No matching row is an expected "not found", not a failure.
        Err(tokio_rusqlite::Error::Error(tokio_rusqlite::rusqlite::Error::QueryReturnedNoRows)) => return Ok(None),
        // Anything else is a real error — pass it back to the caller.
        Err(e) => return Err(e),
    };

    // Check token birth. If it's within the last hour. If it's not then refresh the access token
    if (Utc::now() - user_record.token_birth).num_hours() > 1 {
        //refresh token
    }

    Ok(Some(user_record.spot_token))
}

pub async fn create_user_credential(db_ctx: &db::DBContext, user: serenity::model::user::User) {

}

/// refresh the access token and save it to the db
pub async fn update_user_credential(db_ctx: &db::DBContext, user: db::User) {

}