use std::{sync::LazyLock, time::Duration};

use reqwest::{header::HeaderMap, Client};

pub static SHARED_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .user_agent("pkgforge/soar")
        .build()
        .expect("failed to build default client")
});

pub struct ClientConfig {
    pub user_agent: Option<String>,
    pub headers: Option<HeaderMap>,
    pub proxy: Option<String>,
    pub timeout: Option<Duration>,
}

impl ClientConfig {
    pub fn build(self) -> Result<Client, reqwest::Error> {
        let mut builder = Client::builder();

        if let Some(user_agent) = self.user_agent {
            builder = builder.user_agent(user_agent);
        }

        if let Some(headers) = self.headers {
            builder = builder.default_headers(headers);
        }

        if let Some(proxy_url) = self.proxy {
            builder = builder.proxy(reqwest::Proxy::all(&proxy_url)?);
        }

        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }

        builder.build()
    }
}
