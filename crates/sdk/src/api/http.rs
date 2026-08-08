use base64::{Engine as _, engine::general_purpose};
use reqwest::{
    Client,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::api::types::{ApiBaseConfig, ApiResult, ApiSdkError, AuthConfig};

#[derive(Debug, Clone)]
pub struct HttpClient {
    client: Client,
    base_config: ApiBaseConfig,
}

impl HttpClient {
    pub fn new(base_config: ApiBaseConfig) -> Self {
        Self { client: Client::new(), base_config }
    }

    fn build_url(&self, endpoint: &str) -> String {
        format!(
            "{}/{}",
            self.base_config.server_url.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        )
    }

    fn build_headers(&self, additional_headers: Option<HeaderMap>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        match &self.base_config.auth {
            AuthConfig::BasicAuth { username, password } => {
                let credentials = format!("{}:{}", username, password);
                let encoded = general_purpose::STANDARD.encode(credentials);
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Basic {}", encoded)).unwrap(),
                );
            }
            AuthConfig::ApiKey { api_key } => {
                let header_name = HeaderName::from_static("x-rrelayer-api-key");
                headers.insert(header_name, HeaderValue::from_str(api_key).unwrap());
            }
        }

        if let Some(additional) = additional_headers {
            for (key, value) in additional {
                if let Some(key) = key {
                    headers.insert(key, value);
                }
            }
        }

        headers
    }

    fn handle_response_status(&self, response: &reqwest::Response) -> ApiResult<()> {
        match response.status().as_u16() {
            401 => Err(ApiSdkError::AuthError("Unauthorized".to_string())),
            403 => Err(ApiSdkError::AuthError("Forbidden".to_string())),
            429 => Err(ApiSdkError::RateLimitError),
            _ => Ok(()),
        }
    }

    pub async fn get<T>(&self, endpoint: &str) -> ApiResult<T>
    where
        T: DeserializeOwned,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.get(&url).headers(headers).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    pub async fn get_or_none<T>(&self, endpoint: &str) -> ApiResult<Option<T>>
    where
        T: DeserializeOwned,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.get(&url).headers(headers).send().await?;

        if response.status() == 404 {
            return Ok(None);
        }

        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;
        Ok(response.json::<Option<T>>().await?)
    }

    pub async fn get_with_query<T, Q>(&self, endpoint: &str, query: Option<Q>) -> ApiResult<T>
    where
        T: DeserializeOwned,
        Q: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let mut request = self.client.get(&url).headers(headers);
        if let Some(q) = query {
            request = request.query(&q);
        }

        let response = request.send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;
        Ok(response.json().await?)
    }

    pub async fn post<T, B>(&self, endpoint: &str, body: &B) -> ApiResult<T>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.post(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    pub async fn post_with_headers<T, B>(
        &self,
        endpoint: &str,
        body: &B,
        headers: HeaderMap,
    ) -> ApiResult<T>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(Some(headers));

        let response = self.client.post(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    pub async fn post_status<B>(&self, endpoint: &str, body: &B) -> ApiResult<()>
    where
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.post(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        response.error_for_status()?;

        Ok(())
    }

    pub async fn put<T, B>(&self, endpoint: &str, body: &B) -> ApiResult<T>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.put(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn put_with_headers<T, B>(
        &self,
        endpoint: &str,
        body: &B,
        headers: HeaderMap,
    ) -> ApiResult<T>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(Some(headers));

        let response = self.client.put(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    pub async fn put_status<B>(&self, endpoint: &str, body: &B) -> ApiResult<()>
    where
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.put(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        response.error_for_status()?;

        Ok(())
    }

    pub async fn delete<T>(&self, endpoint: &str) -> ApiResult<T>
    where
        T: DeserializeOwned,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.delete(&url).headers(headers).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn delete_status(&self, endpoint: &str) -> ApiResult<()> {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.delete(&url).headers(headers).send().await?;
        self.handle_response_status(&response)?;
        response.error_for_status()?;

        Ok(())
    }

    pub async fn delete_with_body<T, B>(&self, endpoint: &str, body: &B) -> ApiResult<T>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.delete(&url).headers(headers).json(body).send().await?;
        self.handle_response_status(&response)?;
        let response = response.error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn get_status(&self, endpoint: &str) -> ApiResult<()> {
        let url = self.build_url(endpoint);
        let headers = self.build_headers(None);

        let response = self.client.get(&url).headers(headers).send().await?;
        self.handle_response_status(&response)?;
        response.error_for_status()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct TestPayload {
        value: String,
    }

    fn client_for_response(status_line: &'static str, body: &'static str) -> HttpClient {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test HTTP listener");
        let addr = listener.local_addr().expect("read listener address");

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test HTTP request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).expect("read test HTTP request");
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("write test HTTP response");
        });

        HttpClient::new(ApiBaseConfig {
            server_url: format!("http://{addr}"),
            auth: AuthConfig::BasicAuth {
                username: "user".to_string(),
                password: "password".to_string(),
            },
        })
    }

    #[tokio::test]
    async fn get_or_none_treats_404_as_none() {
        let client = client_for_response("HTTP/1.1 404 Not Found", "not found");

        let result = client
            .get_or_none::<TestPayload>("transactions/external/missing")
            .await
            .expect("404 should be a missing value");

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn get_or_none_treats_json_null_as_none() {
        let client = client_for_response("HTTP/1.1 200 OK", "null");

        let result = client
            .get_or_none::<TestPayload>("transactions/external/missing")
            .await
            .expect("200 null should be a missing value");

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn get_or_none_deserializes_json_object_as_some() {
        let client = client_for_response("HTTP/1.1 200 OK", r#"{"value":"present"}"#);

        let result = client
            .get_or_none::<TestPayload>("transactions/external/present")
            .await
            .expect("object should deserialize");

        assert_eq!(result, Some(TestPayload { value: "present".to_string() }));
    }
}
