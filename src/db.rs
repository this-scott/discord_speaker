use tokio_rusqlite::{Connection, Result};
use chrono::{DateTime, Utc};
#[derive(Clone)]
pub struct DBContext {
    conn: Connection,
}
pub(crate) struct User {
    pub(crate) disc_name: String,
    pub(crate) disc_id: u64,
    pub(crate) spot_token: String,
    pub(crate) spot_refresh: String,
    pub(crate) token_birth: DateTime<Utc>
}

impl DBContext {
    /// Create db connection/context to sqllite server
    pub async fn new(path: String) -> Result<Self> {
        // create connection. Automatically creates is path doesn't exist
        let conn = Connection::open(path).await?;
        // execute PRAGMA statements on the inner rusqlite connection via .call
        let _ = conn.call(|c| c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS User (
                 disc_id      INTEGER PRIMARY KEY,
                 disc_name    TEXT NOT NULL,
                 spot_token   TEXT NOT NULL,
                 spot_refresh TEXT NOT NULL,
                 token_birth DATETIME NOT NULL
             );"
        )).await?;
        Ok(Self {conn})
    }

    /// Get a user row by discord id.
    /// Returns `Err(QueryReturnedNoRows)` if the id is not present; the caller maps that to "not found".
    pub async fn lookup_user(&self, disc_id: u64) -> Result<User> {
        let user = self.conn.call(move |conn| {
            conn.query_row(
                "SELECT disc_name, disc_id, spot_token, spot_refresh, token_birth FROM User WHERE disc_id=?1",
                [disc_id],
                |row| {
                    let token_birth_str: String = row.get(4)?;
                    Ok(User {
                        disc_name: row.get(0)?,
                        disc_id: row.get(1)?,
                        spot_token: row.get(2)?,
                        spot_refresh: row.get(3)?,
                        token_birth: DateTime::parse_from_rfc3339(&token_birth_str)
                            .unwrap()
                            .with_timezone(&Utc),
                    })
                },
            )
        }).await?;
        Ok(user)
    }

}



//todo: add a rotate function to remove the last used key after a certain limit(25)