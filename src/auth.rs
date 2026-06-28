use std::{collections::HashMap, time::Duration};

use crate::db;
use chrono::{DateTime, Utc};
use reqwest::{Client};

pub struct AuthHandler {
    db_ctx: db::DBContext,
    spotify_client_id: String,
    spotify_client_secret: String,
    reqwest_client: Client
}

impl AuthHandler {

    pub async fn new(spotify_client_id: String, spotify_client_secret: String, _cache_location: String) -> Result<Self, Box<dyn std::error::Error>> {
        let db_ctx = db::DBContext::new(_cache_location).await?;
        let reqwest_client = Client::builder().timeout(Duration::from_secs(10)).build()?;
        Ok(Self {db_ctx, spotify_client_id, spotify_client_secret, reqwest_client})
    }

    /// Looks up user against sqllite db to get and apply refresh token. Returns user token or creates one if none found
    pub async fn get_user_credential(&self, user: &serenity::model::user::User) -> Result<Option<String>, tokio_rusqlite::Error> {
        println!("User details: {:?}", user);
        let user_record = match self.db_ctx.lookup_user(user.id.get()).await {
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

    pub async fn create_user_credential(&self, user: serenity::model::user::User) {

        //todo: setup oauth handler for this
    }

    /// refresh the access token and save it to the db
    pub async fn refresh_user_token(&self, user: db::User) -> Result<(), Box<dyn std::error::Error>> {

        let mut form_data = HashMap::new();
            form_data.insert("grant_type", "refresh_token");
            form_data.insert("refresh_token", &user.spot_refresh); 

        let response = self.reqwest_client.post("https://accounts.spotify.com/api/token")
            .basic_auth(self.spotify_client_id.clone(), Some(self.spotify_client_secret.clone()))
            .form(&form_data)
            .send()
            .await?;

        //todo: write response token to database
        Ok(())
    }
}