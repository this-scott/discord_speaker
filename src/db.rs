use tokio_rusqlite::{Connection, Result};

pub struct DBContext {
    conn: Connection,
}
struct User {
    disc_name: String,
    disc_id: u32,
    spot_token: String,
    spot_refresh: String
}

impl DBContext {
    /// Create db connection/context to sqllite server
    pub async fn new(path: String) -> Result<Self> {
        // create connection. Automatically creates is path doesn't exist
        let conn = Connection::open(path).await?;
        Ok(Self {conn})
    }
}


/// Get user credential from database. Create new entry if discord id is not found
pub async fn lookup_user(conn: Connection) -> Result<String> {
    let user = conn.call(|conn| {
        conn.prepare(sql)
    })

    return Ok(token)
}

//todo: add a rotate function to remove the last used key after a certain limit(25)