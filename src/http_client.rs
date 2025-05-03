use std::{
    str::FromStr,
    sync::{Arc, LazyLock, RwLock},
};

use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client,
};

struct SharedClient {
    client: Client,
    config: ClientConfig,
}

static SHARED_CLIENT_STATE: LazyLock<Arc<RwLock<SharedClient>>> = LazyLock::new(|| {
    let config = ClientConfig::default();
    let client = config.build().expect("failed to build default client");

    Arc::new(RwLock::new(SharedClient { client, config }))
});

pub static SHARED_CLIENT: LazyLock<Client> =
    LazyLock::new(|| SHARED_CLIENT_STATE.read().unwrap().client.clone());

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub user_agent: Option<String>,
    pub headers: Option<HeaderMap>,
    pub proxy: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            user_agent: Some("pkgforge/soar".to_string()),
            headers: None,
            proxy: None,
        }
    }
}

impl ClientConfig {
    pub fn build(&self) -> Result<Client, reqwest::Error> {
        let mut builder = Client::builder();

        if let Some(user_agent) = &self.user_agent {
            builder = builder.user_agent(user_agent);
        }

        if let Some(headers) = &self.headers {
            builder = builder.default_headers(headers.clone());
        }

        if let Some(proxy_url) = &self.proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
        }

        builder.build()
    }
}

pub fn create_http_header_map(headers: Vec<String>) -> HeaderMap {
    let mut header_map = HeaderMap::new();

    for header in headers {
        let parts: Vec<&str> = header.splitn(2, ':').collect();
        if parts.len() == 2 {
            let key = parts[0].trim();
            let value = parts[1].trim();

            if let Ok(header_name) = HeaderName::from_str(key) {
                if let Ok(header_value) = HeaderValue::from_str(value) {
                    header_map.insert(header_name, header_value);
                }
            }
        }
    }

    header_map
}

pub fn configure_http_client<F>(updater: F) -> Result<(), reqwest::Error>
where
    F: FnOnce(&mut ClientConfig),
{
    let mut state = SHARED_CLIENT_STATE.write().unwrap();
    let mut new_config = state.config.clone();

    updater(&mut new_config);

    let new_client = new_config.build()?;

    state.client = new_client;
    state.config = new_config;

    Ok(())
}
