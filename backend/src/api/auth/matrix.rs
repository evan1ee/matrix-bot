use matrix_sdk::ruma::{ServerName, UserId};
use matrix_sdk::Client;
use serde::{Deserialize, Serialize};

use crate::api::auth::types::{ApiState, Claims, JwtKeys};

pub async fn get_bot_matrix_access_token(state: &ApiState) -> Option<String> {
    if let Some(tok) = state.matrix_access_token.read().await.clone() {
        return Some(tok);
    }

    if let Ok(file_tok) = std::fs::read_to_string("data/matrix_token.json") {
        let trimmed = file_tok.trim().to_string();
        if !trimmed.is_empty() {
            *state.matrix_access_token.write().await = Some(trimmed.clone());
            return Some(trimmed);
        }
    }

    dotenvy::dotenv().ok();
    let env_user = std::env::var("MATRIX_USERNAME").ok()?;
    let env_pass = std::env::var("MATRIX_PASSWORD").ok()?;

    let client = get_matrix_sdk_client(&state.homeserver).await;
    let hs_url = if let Some(c) = client {
        c.homeserver().to_string()
    } else {
        state.homeserver.clone()
    };
    let clean_hs = hs_url.trim_end_matches('/');
    let clean_domain = state
        .homeserver
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let full_user_id = if env_user.starts_with('@') {
        env_user.clone()
    } else {
        format!("@{}:{}", env_user, clean_domain)
    };

    let login_url = format!("{}/_matrix/client/v3/login", clean_hs);

    #[derive(Serialize)]
    struct LoginReq<'a> {
        #[serde(rename = "type")]
        login_type: &'a str,
        identifier: UserIdentifier<'a>,
        password: &'a str,
    }

    #[derive(Serialize)]
    struct UserIdentifier<'a> {
        #[serde(rename = "type")]
        id_type: &'a str,
        user: &'a str,
    }

    let payload = LoginReq {
        login_type: "m.login.password",
        identifier: UserIdentifier {
            id_type: "m.id.user",
            user: &full_user_id,
        },
        password: &env_pass,
    };

    match state.http_client.post(&login_url).json(&payload).send().await {
        Ok(res) => {
            tracing::info!("Bot login request to {}: status={}", login_url, res.status());
            if res.status().is_success() {
                #[derive(Deserialize)]
                struct LoginResp {
                    access_token: String,
                }
                if let Ok(data) = res.json::<LoginResp>().await {
                    tracing::info!("Successfully obtained bot access_token from homeserver!");
                    std::fs::create_dir_all("data").ok();

                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        let mut options = std::fs::OpenOptions::new();
                        options.write(true).create(true).truncate(true).mode(0o600);
                        if let Ok(mut f) = options.open("data/matrix_token.json") {
                            use std::io::Write;
                            let _ = f.write_all(data.access_token.as_bytes());
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        std::fs::write("data/matrix_token.json", &data.access_token).ok();
                    }

                    *state.matrix_access_token.write().await = Some(data.access_token.clone());
                    return Some(data.access_token);
                }
            }
        }
        Err(err) => {
            tracing::warn!("Bot login HTTP error: {:?}", err);
        }
    }

    None
}

pub async fn get_matrix_sdk_client(homeserver_input: &str) -> Option<Client> {
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    if let Ok(server_name) = ServerName::parse(clean_domain) {
        if let Ok(c) = Client::builder().server_name(&server_name).build().await {
            return Some(c);
        }
    }

    let fallback_url = if homeserver_input.starts_with("http://")
        || homeserver_input.starts_with("https://")
    {
        homeserver_input.to_string()
    } else {
        format!("https://{}", clean_domain)
    };

    Client::builder().homeserver_url(&fallback_url).build().await.ok()
}

pub async fn validate_matrix_access_token(state: &ApiState, token: &str) -> Option<String> {
    let client = get_matrix_sdk_client(&state.homeserver).await?;
    let hs_url = client.homeserver().to_string();
    let whoami_url = format!("{}/_matrix/client/v3/account/whoami", hs_url.trim_end_matches('/'));

    let res = state
        .http_client
        .get(&whoami_url)
        .bearer_auth(token)
        .send()
        .await
        .ok()?;

    if res.status().is_success() {
        #[derive(Deserialize)]
        struct WhoAmI {
            user_id: String,
        }
        let data: WhoAmI = res.json().await.ok()?;
        Some(data.user_id)
    } else {
        None
    }
}

pub async fn authenticate_matrix_user(
    state: &ApiState,
    username: &str,
    password: &str,
) -> Option<(String, String)> {
    let homeserver_input = &state.homeserver;
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let user_id = if username.starts_with('@') {
        UserId::parse(username).ok()?
    } else {
        let raw_id = format!("@{}:{}", username, clean_domain);
        UserId::parse(&raw_id).ok()?
    };

    let client = get_matrix_sdk_client(&state.homeserver).await?;

    match client.matrix_auth().login_username(&user_id, password).send().await {
        Ok(res) => Some((res.user_id.to_string(), res.access_token)),
        Err(err) => {
            tracing::warn!("Matrix authentication failed for {}: {:?}", username, err);
            None
        }
    }
}

pub fn generate_jwt(username: &str, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: username.to_string(),
        exp: expiration,
    };

    let keys = JwtKeys::new(secret.as_bytes());
    jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &keys.encoding)
}
