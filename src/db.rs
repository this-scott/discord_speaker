use tokio_rusqlite::{params, Connection, OptionalExtension, Result};
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

pub(crate) struct Session {
    pub(crate) disc_name: String,
    pub(crate) disc_id: u64,
    pub(crate) state: String
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
                    id INTEGER PRIMARY KEY,
                    disc_id      INTEGER NOT NULL,
                    disc_name    TEXT NOT NULL,
                    spot_token   TEXT NOT NULL,
                    spot_refresh TEXT NOT NULL,
                    token_birth DATETIME NOT NULL
                );
                CREATE TABLE IF NOT EXISTS AuthSessions (
                    id INTEGER PRIMARY KEY,
                    state VARCHAR(16) NOT NULL,
                    disc_id INTEGER NOT NULL,
                    disc_name TEXT NOT NULL
                );"
        )).await?;
        Ok(Self {conn})
    }

    /// Get a user row by discord id.
    /// Returns `Err(QueryReturnedNoRows)` if the id is not present; the caller maps that to "not found".
    pub async fn get_user(&self, disc_id: u64) -> Result<User> {
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

    /// Look up an auth session by state. Returns `Ok(None)` if no matching row exists.
    pub async fn get_session(&self, state: &String) -> Result<Option<Session>> {
        let state = state.to_owned();
        let session = self.conn.call(move |conn| {
            conn.query_one(
                "SELECT state, disc_id, disc_name FROM AuthSessions WHERE state=?1",
                [state],
                |row| {
                    Ok(Session {
                        state: row.get(0)?,
                        disc_id: row.get(1)?,
                        disc_name: row.get(2)?,
                    })
                },
            )
            .optional()
        }).await?;

        Ok(session)
    }

    pub async fn create_user(&self, user: User) -> Result<()> {
        self.conn.call( move |conn| {
            conn.execute(
                "INSERT INTO User (disc_id, disc_name, spot_token, spot_refresh, token_birth) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![user.disc_id, user.disc_name, user.spot_token, user.spot_refresh, user.token_birth.to_rfc3339()],
            )
        }).await?;
        Ok(())
    }

    pub async fn create_session(&self, state: &String, disc_id: u64, disc_name: String) -> Result<()> {
        let state = state.to_owned();
        self.conn.call( move |conn| {
            conn.execute("INSERT INTO AuthSessions (state, disc_id, disc_name) VALUES (?1, ?2, ?3)", [state, disc_id.to_string(), disc_name])
        }).await?;
        Ok(())
    }

    /// Remove auth session for callback complete.
    pub async fn delete_session(&self, state: &String) -> Result<()> {
        let state = state.to_owned();
        self.conn.call( move |conn| {
            conn.execute("DELETE FROM AuthSessions WHERE state=?1", [state])
        }).await?;
        Ok(())
    }

    // update user and return it's token
    pub async fn update_user(&self, user: User) -> Result<()> {
        self.conn.call( move |conn| {
            conn.execute("UPDATE User SET spot_token=?1, spot_refresh=?2 WHERE disc_id=?3", params![user.spot_token,user.spot_refresh,user.disc_id])
        }).await?;
        Ok(())
    }

    
}

//todo: add a rotate function to remove the last used key after a certain limit(25)