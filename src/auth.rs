use std::{collections::HashMap, time::Duration};

use crate::db::{self};
use chrono::{Utc};
// these two are blending technically blending domain and application layer but we're so small it doesn't matter
// Arc and Axum use Arc so they're not creating new connections when cloned
use reqwest::{Client};
use axum::{Router, extract::{Query, State}, routing::get, response::Html, http::StatusCode};
use serde::Deserialize;
use tokio::time::{sleep};

pub struct AuthHandler {
    db_ctx: db::DBContext,
    spotify_client_id: String,
    spotify_client_secret: String,
    redirect_uri: String,
    reqwest_client: Client,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CallbackParams {
    Success {
        code: String,
        state: String
    },
    Failure {
        error: String,
        state: String
    },
}

// I really think this is too much for a single callback but axum handlers can't import self
#[derive(Clone)]
struct CallbackCtx {
    db_ctx: db::DBContext,
    reqwest_client: Client,
    spotify_client_id: String,
    spotify_client_secret: String,
    redirect_uri: String,
}

#[derive(Deserialize, Debug)]
struct ExchangeResponse {
    access_token: String,
    // Refresh tokens don't refresh so I need to keep them and expect them not to appear after first exchange
    refresh_token: Option<String>
}

impl AuthHandler {

    pub async fn new(spotify_client_id: String, spotify_client_secret: String, redirect_uri: String, _cache_location: String) -> Result<Self, Box<dyn std::error::Error>> {
        let db_ctx = db::DBContext::new(_cache_location).await?;
        let reqwest_client = Client::builder().timeout(Duration::from_secs(10)).build()?;
        Ok(Self {db_ctx, spotify_client_id, spotify_client_secret, redirect_uri, reqwest_client})
    }

    /// Building the router here because it has to listen outside of auth but I don't want to untangle it from auth
    pub fn router(&self) -> Router {
        Router::new()
            .route("/token", get(Self::oauth_callback))
            .with_state(CallbackCtx {
                db_ctx: self.db_ctx.clone(),
                reqwest_client: self.reqwest_client.clone(),
                spotify_client_id: self.spotify_client_id.clone(),
                spotify_client_secret: self.spotify_client_secret.clone(),
                redirect_uri: self.redirect_uri.clone(),
            })
    }

    /// Looks up user against sqllite db to get and apply refresh token. Returns user token or creates one if none found
    pub async fn get_user_credential(&self, user: &serenity::model::user::User) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        println!("User details: {:?}", user);
        let user_record = match self.db_ctx.get_user(user.id.get()).await {
            Ok(user_record) => user_record,
            // No matching row is an expected "not found", not a failure.
            Err(tokio_rusqlite::Error::Error(tokio_rusqlite::rusqlite::Error::QueryReturnedNoRows)) => return Ok(None),
            // Anything else is a real error — pass it back to the caller.
            Err(e) => return Err(e.into()),
        };

        // Check token birth. If it's within the last hour. If it's not then refresh the access token
        if (Utc::now() - user_record.token_birth).num_hours() > 1 {
            // refresh dead token
            let access_token = self.refresh_user_token(user_record).await?;
            return Ok(Some(access_token));
        }

        Ok(Some(user_record.spot_token))
    }

    pub async fn create_token_request(&self, state: &String, user: &serenity::model::user::User) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        //create token session with discord information
        self.db_ctx.create_session(state, user.id.get(), user.name.to_string()).await?;

        // check for movement every two seconds
        let mut attempts = 0;
        while attempts < 60 {
            // session still present means the callback hasn't cleared it yet — keep waiting
            if self.db_ctx.get_session(state).await?.is_some() {
                attempts += 1;
                sleep(Duration::from_secs(2)).await;
                continue;
            }

            // session was cleared by the callback — the credential should now exist
            if let Some(token) = self.get_user_credential(user).await? {
                println!("Token received at {token}");
                return Ok(token);
            }
            return Ok("".to_string());
        }

        // timeout
        self.db_ctx.delete_session(state).await?;
        Ok("".to_string())
    }

    /// refresh the access token and save it to the db
    pub async fn refresh_user_token(&self, user: db::User) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {

        let mut form_data = HashMap::new();
            form_data.insert("grant_type", "refresh_token");
            form_data.insert("refresh_token", &user.spot_refresh); 

        let response = self.reqwest_client.post("https://accounts.spotify.com/api/token")
            .basic_auth(self.spotify_client_id.clone(), Some(self.spotify_client_secret.clone()))
            .form(&form_data)
            .send()
            .await?;

        let new_access_token = match response.json::<ExchangeResponse>().await {
            Ok(new_access_token) => new_access_token,
            Err(e) => {
                    eprintln!("Failed to extract json: {e:?}");
                    return Err(e.into());
            }
        };
        // create user in database
        let disc_name = user.disc_name.clone();
        let access_token = new_access_token.access_token.clone();
        // handling optional refresh token 
        let spot_refresh = new_access_token.refresh_token.unwrap_or(user.spot_refresh);
        let user = db::User {
            disc_id: user.disc_id,
            disc_name: user.disc_name,
            spot_token: new_access_token.access_token,
            spot_refresh,
            token_birth: Utc::now()
        };
        match self.db_ctx.update_user(user).await {
            Ok(()) => (),
            Err(e) => {
                eprintln!("Failed to refresh token for user {disc_name}: {e:?}");
                return Err(e.into());
            }
        };
        Ok(access_token)
    }

    // handles oauth tokens granted by idp and given to server from client
    async fn oauth_callback(State(ctx): State<CallbackCtx>, Query(params): Query<CallbackParams>) -> (StatusCode, Html<&'static str>) {
        match params {
            CallbackParams::Success { code, state } => {
                println!("Session {state} received success code");

                // callback to spotify for access token then throw it in db
                let session = match ctx.db_ctx.get_session(&state).await {
                    Ok(Some(session)) => session,
                    Ok(None) => {
                        eprintln!("Callback for state {state} but no matching session found");
                        return (StatusCode::NOT_FOUND, Html("<h1>Session not found</h1><p>This authorization link has expired. Please try again from Discord.</p>"));
                    }
                    Err(e) => {
                        eprintln!("DB error looking up session {state}: {e:?}");
                        return (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>Something went wrong</h1><p>Please try again later.</p>"));
                    }
                };

                // make request to spotify for access token
                let mut form_data = HashMap::new();
                    form_data.insert("grant_type", "authorization_code");
                    form_data.insert("code", &code);
                    form_data.insert("redirect_uri", &ctx.redirect_uri);

                let response = match ctx.reqwest_client.post("https://accounts.spotify.com/api/token")
                    .basic_auth(ctx.spotify_client_id.clone(), Some(ctx.spotify_client_secret.clone()))
                    .form(&form_data)
                    .send()
                    .await {
                        Ok(response) => response,
                        Err(e) => {
                            eprintln!("Token exchange request faled for session {state}: {e:?}");
                            return (StatusCode::BAD_GATEWAY, Html("<h1>Could not reach Spotify</h1><p>Please try again later.</p>"));
                        }
                    };

                //extract access token, refresh token from reponse
                let new_access_token = match response.json::<ExchangeResponse>().await {
                    Ok(new_access_token) => new_access_token,
                    Err(e) => {
                            eprintln!("Failed to extract json: {e:?}");
                            return (StatusCode::BAD_GATEWAY, Html("<h1>Unexpected response from Spotify</h1><p>Please try again later.</p>"));
                    }
                };

                let spot_refresh = match new_access_token.refresh_token {
                    Some(spot_refresh) => spot_refresh,
                    None => {
                        eprintln!("Spotify returned no refresh_token for session {state}");
                        return (StatusCode::BAD_GATEWAY, Html("<h1>Unexpected response from Spotify</h1><p>Please try again later.</p>"));
                    }
                };

                // create user in database
                let user = db::User {
                    disc_id: session.disc_id,
                    disc_name: session.disc_name,
                    spot_token: new_access_token.access_token,
                    spot_refresh,
                    token_birth: Utc::now()
                };
                match ctx.db_ctx.create_user(user).await {
                    Ok(()) => (),
                    Err(e) => {
                        eprintln!("Failed to create db object for session {state}: {e:?}");
                        return (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>Auth save error</h1><p>Please try again later.</p>"));
                    }
                };

                // callback successful. End auth session
                if let Err(e) = ctx.db_ctx.delete_session(&state).await {
                    eprintln!("Failed to clear session {state} after auth: {e:?}");
                }

                (StatusCode::OK, Html("<h1>Success</h1><p>Spotify is linked. Close this tab and head into spotify.</p>"))
            }
            CallbackParams::Failure { error, state } => {
                eprintln!("Session {state} failed to authorize: {error:?}");
                (StatusCode::BAD_REQUEST, Html("<h1>Cancelled</h1><p>Close this tab and try again from Discord.</p>"))
            }
        }
    }

}

