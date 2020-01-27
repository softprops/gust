//! Hubcaps provides a set of building blocks for interacting with the Github API
//!
//! # Examples
//!
//!  Typical use will require instantiation of a Github client. Which requires
//! a user agent string and set of `hubcaps::Credentials`.
//!
//! ```no_run
//! use hubcaps::{Credentials, Github};
//!
//! fn main() {
//!   let github = Github::new(
//!     String::from("user-agent-name"),
//!     Credentials::Token(
//!       String::from("personal-access-token")
//!     ),
//!   );
//! }
//! ```
//!
//! Github enterprise users will want to create a client with the
//! [Github#host](struct.Github.html#method.host) method
//!
//! Access to various services are provided via methods on instances of the `Github` type.
//!
//! The convention for executing operations typically looks like
//! `github.repo(.., ..).service().operation(OperationOptions)` where operation may be `create`,
//! `delete`, etc.
//!
//! Services and their types are packaged under their own module namespace.
//! A service interface will provide access to operations and operations may access options types
//! that define the various parameter options available for the operation. Most operation option
//! types expose `builder()` methods for a builder oriented style of constructing options.
//!
//! ## Entity listings
//!
//! Many of Github's APIs return a collection of entities with a common interface for supporting pagination
//! Hubcaps supports two types of interfaces for working with listings. `list(...)` interfaces return the first
//! ( often enough ) list of entities. Alternatively for listings that require > 30 items you may wish to
//! use the `iter(..)` variant which returns a `futures::Stream` over all entities in a paginated set.
//!
//! # Errors
//!
//! Operations typically result in a `hubcaps::Future` with an error type pinned to
//! [hubcaps::Error](errors/struct.Error.html).
//!
//! ## Rate Limiting
//!
//! A special note should be taken when accounting for Github's
//! [API Rate Limiting](https://developer.github.com/v3/rate_limit/)
//! A special case
//! [hubcaps::ErrorKind::RateLimit](errors/enum.ErrorKind.html#variant.RateLimit)
//! will be returned from api operations when the rate limit
//! associated with credentials has been exhausted. This type will include a reset
//! Duration to wait before making future requests.
//!
//! This crate uses the `log` crate's debug log interface to log x-rate-limit
//! headers received from Github.
//! If you are attempting to test your access patterns against
//! Github's rate limits, enable debug looking and look for "x-rate-limit"
//! log patterns sourced from this crate
//!
//! # Features
//!
//! ## httpcache
//!
//! Github supports conditional HTTP requests using etags to checksum responses
//! Experimental support for utilizing this to cache responses locally with the
//! `httpcache` feature flag
//!
//! To enable this, add the following to your `Cargo.toml` file
//!
//! ```toml
//! [dependencies.hubcaps]
//!  version = "..."
//!  default-features = false
//!  features = ["default-tls","httpcache"]
//! ```
//!
//! Then use the `Github::custom` constructor to provide a cache implementation. See
//! the conditional_requests example in this crates github repository for an example usage
//!
#![allow(missing_docs)] // todo: make this a deny eventually

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{Future as FFuture, Stream as FStream};
#[cfg(feature = "httpcache")]
use http::header::IF_NONE_MATCH;
use http::header::{HeaderMap, HeaderValue};
use http::header::{ACCEPT, AUTHORIZATION, ETAG, LINK, USER_AGENT};
use http::{Method, StatusCode};
use hyperx::header::{qitem, Link};
#[cfg(feature = "httpcache")]
use hyperx::header::{LinkValue, RelationType};
use jsonwebtoken as jwt;
use log::{debug, error, trace};
use mime::Mime;
use reqwest::{Body, Client, Url};
use serde::de::DeserializeOwned;
use serde::Serialize;

#[doc(hidden)] // public for doc testing and integration testing only
#[cfg(feature = "httpcache")]
pub mod http_cache;
#[macro_use]
mod macros; // expose json! macro to child modules
pub mod activity;
pub mod app;
pub mod branches;
pub mod checks;
pub mod collaborators;
pub mod comments;
pub mod content;
pub mod deployments;
pub mod errors;
pub mod gists;
pub mod git;
pub mod hooks;
pub mod issues;
pub mod keys;
pub mod labels;
pub mod membership;
pub mod notifications;
pub mod organizations;
pub mod pull_commits;
pub mod pulls;
pub mod rate_limit;
pub mod releases;
pub mod repo_commits;
pub mod repositories;
pub mod review_comments;
pub mod review_requests;
pub mod search;
pub mod stars;
pub mod statuses;
pub mod teams;
pub mod traffic;
pub mod users;
pub mod watching;

mod unfold;
use unfold::unfold;

pub use crate::errors::{Error, ErrorKind, Result};
#[cfg(feature = "httpcache")]
pub use crate::http_cache::{BoxedHttpCache, HttpCache};

use crate::activity::Activity;
use crate::app::App;
use crate::gists::{Gists, UserGists};
use crate::organizations::{Organization, Organizations, UserOrganizations};
use crate::rate_limit::RateLimit;
use crate::repositories::{OrganizationRepositories, Repositories, Repository, UserRepositories};
use crate::search::Search;
use crate::users::Users;

const DEFAULT_HOST: &str = "https://api.github.com";
// We use 9 minutes for the life to give some buffer for clock drift between
// our clock and GitHub's. The absolute max is 10 minutes.
const MAX_JWT_TOKEN_LIFE: time::Duration = time::Duration::from_secs(60 * 9);
// 8 minutes so we refresh sooner than it actually expires
const JWT_TOKEN_REFRESH_PERIOD: time::Duration = time::Duration::from_secs(60 * 8);

/// A type alias for `Futures` that may return `hubcaps::Errors`
pub type Future<T> = Box<dyn FFuture<Output = std::result::Result<T, Error>> + Send + Sync>;

/// A type alias for `Streams` that may result in `hubcaps::Errors`
pub type Stream<T> = std::pin::Pin<Box<dyn FStream<Item = std::result::Result<T, Error>>>>;

const X_GITHUB_REQUEST_ID: &str = "x-github-request-id";
const X_RATELIMIT_LIMIT: &str = "x-ratelimit-limit";
const X_RATELIMIT_REMAINING: &str = "x-ratelimit-remaining";
const X_RATELIMIT_RESET: &str = "x-ratelimit-reset";

pub(crate) mod utils {
    use hyperx::header::{Link, RelationType};
    pub use percent_encoding::percent_encode;
    use percent_encoding::{AsciiSet, CONTROLS};

    /// https://url.spec.whatwg.org/#fragment-percent-encode-set
    const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

    /// https://url.spec.whatwg.org/#path-percent-encode-set
    pub const PATH: &AsciiSet = &FRAGMENT.add(b'#').add(b'?').add(b'{').add(b'}');

    pub const PATH_SEGMENT: &AsciiSet = &PATH.add(b'/').add(b'%');

    pub fn next_link(l: &Link) -> Option<String> {
        l.values()
            .into_iter()
            .find(|v| v.rel().unwrap_or(&[]).get(0) == Some(&RelationType::Next))
            .map(|v| v.link().to_owned())
    }
}

/// Github defined Media types
/// See [this doc](https://developer.github.com/v3/media/) for more for more information
#[derive(Clone, Copy)]
pub enum MediaType {
    /// Return json (the default)
    Json,
    /// Return json in preview form
    Preview(&'static str),
}

impl Default for MediaType {
    fn default() -> MediaType {
        MediaType::Json
    }
}

impl From<MediaType> for Mime {
    fn from(media: MediaType) -> Mime {
        match media {
            MediaType::Json => "application/vnd.github.v3+json".parse().unwrap(),
            MediaType::Preview(codename) => {
                format!("application/vnd.github.{}-preview+json", codename)
                    .parse()
                    .unwrap_or_else(|_| {
                        panic!("could not parse media type for preview {}", codename)
                    })
            }
        }
    }
}

/// Controls what sort of authentication is required for this request
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AuthenticationConstraint {
    /// No constraint
    Unconstrained,
    /// Must be JWT
    JWT,
}

/// enum representation of Github list sorting options
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortDirection {
    /// Sort in ascending order (the default)
    Asc,
    /// Sort in descending order
    Desc,
}

impl fmt::Display for SortDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SortDirection::Asc => "asc",
            SortDirection::Desc => "desc",
        }
        .fmt(f)
    }
}

impl Default for SortDirection {
    fn default() -> SortDirection {
        SortDirection::Asc
    }
}

/// Various forms of authentication credentials supported by Github
#[derive(Debug, PartialEq, Clone)]
pub enum Credentials {
    /// Oauth token string
    /// https://developer.github.com/v3/#oauth2-token-sent-in-a-header
    Token(String),
    /// Oauth client id and secret
    /// https://developer.github.com/v3/#oauth2-keysecret
    Client(String, String),
    /// JWT token exchange, to be performed transparently in the
    /// background. app-id, DER key-file.
    /// https://developer.github.com/apps/building-github-apps/authenticating-with-github-apps/
    JWT(JWTCredentials),
    /// JWT-based App Installation Token
    /// https://developer.github.com/apps/building-github-apps/authenticating-with-github-apps/
    InstallationToken(InstallationTokenGenerator),
}

/// JSON Web Token authentication mechanism
///
/// The GitHub client methods are all &self, but the dynamically
/// generated JWT token changes regularly. The token is also a bit
/// expensive to regenerate, so we do want to have a mutable cache.
///
/// We use a token inside a Mutex so we can have interior mutability
/// even though JWTCredentials is not mutable.
#[derive(Debug, Clone)]
pub struct JWTCredentials {
    pub app_id: u64,
    /// DER RSA key. Generate with
    /// `openssl rsa -in private_rsa_key.pem -outform DER -out private_rsa_key.der`
    pub private_key: Vec<u8>,
    cache: Arc<Mutex<ExpiringJWTCredential>>,
}

impl JWTCredentials {
    pub fn new(app_id: u64, private_key: Vec<u8>) -> Result<JWTCredentials> {
        let creds = ExpiringJWTCredential::calculate(app_id, &private_key)?;

        Ok(JWTCredentials {
            app_id: app_id,
            private_key: private_key,
            cache: Arc::new(Mutex::new(creds)),
        })
    }

    fn is_stale(&self) -> bool {
        self.cache.lock().unwrap().is_stale()
    }

    /// Fetch a valid JWT token, regenerating it if necessary
    pub fn token(&self) -> String {
        let mut expiring = self.cache.lock().unwrap();
        if expiring.is_stale() {
            *expiring = ExpiringJWTCredential::calculate(self.app_id, &self.private_key)
                .expect("JWT private key worked before, it should work now...");
        }

        expiring.token.clone()
    }
}

impl PartialEq for JWTCredentials {
    fn eq(&self, other: &JWTCredentials) -> bool {
        self.app_id == other.app_id && self.private_key == other.private_key
    }
}

#[derive(Debug)]
struct ExpiringJWTCredential {
    token: String,
    created_at: time::Instant,
}

#[derive(Serialize)]
struct JWTCredentialClaim {
    iat: u64,
    exp: u64,
    iss: u64,
}

impl ExpiringJWTCredential {
    fn calculate(app_id: u64, private_key: &[u8]) -> Result<ExpiringJWTCredential> {
        // SystemTime can go backwards, Instant can't, so always use
        // Instant for ensuring regular cycling.
        let created_at = time::Instant::now();
        let now = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .unwrap();
        let expires = now + MAX_JWT_TOKEN_LIFE;

        let payload = JWTCredentialClaim {
            iat: now.as_secs(),
            exp: expires.as_secs(),
            iss: app_id,
        };
        let header = jwt::Header::new(jwt::Algorithm::RS256);
        let jwt = jwt::encode(
            &header,
            &payload,
            &jwt::EncodingKey::from_rsa_pem(private_key).expect("lol bad key I guess"),
        )?;

        Ok(ExpiringJWTCredential {
            created_at: created_at,
            token: jwt,
        })
    }

    fn is_stale(&self) -> bool {
        self.created_at.elapsed() >= JWT_TOKEN_REFRESH_PERIOD
    }
}

/// A caching token "generator" which contains JWT credentials.
///
/// The authentication mechanism in the GitHub client library
/// determines if the token is stale, and if so, uses the contained
/// JWT credentials to fetch a new installation token.
///
/// The Mutex<Option> access key is for interior mutability.
#[derive(Debug, Clone)]
pub struct InstallationTokenGenerator {
    pub installation_id: u64,
    pub jwt_credential: Box<Credentials>,
    access_key: Arc<Mutex<Option<String>>>,
}

impl InstallationTokenGenerator {
    pub fn new(installation_id: u64, creds: JWTCredentials) -> InstallationTokenGenerator {
        InstallationTokenGenerator {
            installation_id: installation_id,
            jwt_credential: Box::new(Credentials::JWT(creds)),
            access_key: Arc::new(Mutex::new(None)),
        }
    }

    fn token(&self) -> Option<String> {
        if let Credentials::JWT(ref creds) = *self.jwt_credential {
            if creds.is_stale() {
                return None;
            }
        }
        self.access_key.lock().unwrap().clone()
    }

    fn jwt(&self) -> &Credentials {
        &*self.jwt_credential
    }
}

impl PartialEq for InstallationTokenGenerator {
    fn eq(&self, other: &InstallationTokenGenerator) -> bool {
        self.installation_id == other.installation_id && self.jwt_credential == other.jwt_credential
    }
}

/// Entry point interface for interacting with Github API
#[derive(Clone, Debug)]
pub struct Github {
    host: String,
    agent: String,
    client: Client,
    credentials: Option<Credentials>,
    #[cfg(feature = "httpcache")]
    http_cache: BoxedHttpCache,
}

impl Github {
    pub fn new<A, C>(agent: A, credentials: C) -> Result<Self>
    where
        A: Into<String>,
        C: Into<Option<Credentials>>,
    {
        Self::host(DEFAULT_HOST, agent, credentials)
    }

    pub fn host<H, A, C>(host: H, agent: A, credentials: C) -> Result<Self>
    where
        H: Into<String>,
        A: Into<String>,
        C: Into<Option<Credentials>>,
    {
        let http = Client::builder().build()?;
        #[cfg(feature = "httpcache")]
        {
            Ok(Self::custom(
                host,
                agent,
                credentials,
                http,
                HttpCache::noop(),
            ))
        }
        #[cfg(not(feature = "httpcache"))]
        {
            Ok(Self::custom(host, agent, credentials, http))
        }
    }

    #[cfg(feature = "httpcache")]
    pub fn custom<H, A, CR>(
        host: H,
        agent: A,
        credentials: CR,
        http: Client,
        http_cache: BoxedHttpCache,
    ) -> Self
    where
        H: Into<String>,
        A: Into<String>,
        CR: Into<Option<Credentials>>,
    {
        Self {
            host: host.into(),
            agent: agent.into(),
            client: http,
            credentials: credentials.into(),
            http_cache,
        }
    }

    #[cfg(not(feature = "httpcache"))]
    pub fn custom<H, A, CR>(host: H, agent: A, credentials: CR, http: Client) -> Self
    where
        H: Into<String>,
        A: Into<String>,
        CR: Into<Option<Credentials>>,
    {
        Self {
            host: host.into(),
            agent: agent.into(),
            client: http,
            credentials: credentials.into(),
        }
    }

    pub fn set_credentials<CR>(&mut self, credentials: CR)
    where
        CR: Into<Option<Credentials>>,
    {
        self.credentials = credentials.into();
    }

    pub fn rate_limit(&self) -> RateLimit {
        RateLimit::new(self.clone())
    }

    /// Return a reference to user activity
    pub fn activity(&self) -> Activity {
        Activity::new(self.clone())
    }

    /// Return a reference to a Github repository
    pub fn repo<O, R>(&self, owner: O, repo: R) -> Repository
    where
        O: Into<String>,
        R: Into<String>,
    {
        Repository::new(self.clone(), owner, repo)
    }

    /// Return a reference to the collection of repositories owned by and
    /// associated with an owner
    pub fn user_repos<S>(&self, owner: S) -> UserRepositories
    where
        S: Into<String>,
    {
        UserRepositories::new(self.clone(), owner)
    }

    /// Return a reference to the collection of repositories owned by the user
    /// associated with the current authentication credentials
    pub fn repos(&self) -> Repositories {
        Repositories::new(self.clone())
    }

    pub fn org<O>(&self, org: O) -> Organization
    where
        O: Into<String>,
    {
        Organization::new(self.clone(), org)
    }

    /// Return a reference to the collection of organizations that the user
    /// associated with the current authentication credentials is in
    pub fn orgs(&self) -> Organizations {
        Organizations::new(self.clone())
    }

    /// Return a reference to an interface that provides access
    /// to user information.
    pub fn users(&self) -> Users {
        Users::new(self.clone())
    }

    /// Return a reference to the collection of organizations a user
    /// is publicly associated with
    pub fn user_orgs<U>(&self, user: U) -> UserOrganizations
    where
        U: Into<String>,
    {
        UserOrganizations::new(self.clone(), user)
    }

    /// Return a reference to an interface that provides access to a user's gists
    pub fn user_gists<O>(&self, owner: O) -> UserGists
    where
        O: Into<String>,
    {
        UserGists::new(self.clone(), owner)
    }

    /// Return a reference to an interface that provides access to the
    /// gists belonging to the owner of the token used to configure this client
    pub fn gists(&self) -> Gists {
        Gists::new(self.clone())
    }

    /// Return a reference to an interface that provides access to search operations
    pub fn search(&self) -> Search {
        Search::new(self.clone())
    }

    /// Return a reference to the collection of repositories owned by and
    /// associated with an organization
    pub fn org_repos<O>(&self, org: O) -> OrganizationRepositories
    where
        O: Into<String>,
    {
        OrganizationRepositories::new(self.clone(), org)
    }

    /// Return a reference to GitHub Apps
    pub fn app(&self) -> App {
        App::new(self.clone())
    }

    fn credentials(&self, authentication: AuthenticationConstraint) -> Option<&Credentials> {
        match (authentication, self.credentials.as_ref()) {
            (AuthenticationConstraint::Unconstrained, creds) => creds,
            (AuthenticationConstraint::JWT, creds @ Some(&Credentials::JWT(_))) => creds,
            (
                AuthenticationConstraint::JWT,
                Some(&Credentials::InstallationToken(ref apptoken)),
            ) => Some(apptoken.jwt()),
            (AuthenticationConstraint::JWT, creds) => {
                error!(
                    "Request needs JWT authentication but only {:?} available",
                    creds
                );
                None
            }
        }
    }

    async fn url_and_auth(
        &self,
        uri: &str,
        authentication: AuthenticationConstraint,
    ) -> Result<(Url, Option<String>)> {
        let parsed_url = uri.parse::<Url>().map_err(Error::from)?;

        let tuple = match self.credentials(authentication) {
            Some(&Credentials::Client(ref id, ref secret)) => {
                let mut parsed_url = parsed_url;
                parsed_url
                    .query_pairs_mut()
                    .append_pair("client_id", id)
                    .append_pair("client_secret", secret);
                (parsed_url, None)
            }
            Some(&Credentials::Token(ref token)) => {
                let auth = format!("token {}", token);
                (parsed_url, Some(auth))
            }
            Some(&Credentials::JWT(ref jwt)) => {
                let auth = format!("Bearer {}", jwt.token());
                (parsed_url, Some(auth))
            }
            Some(&Credentials::InstallationToken(ref apptoken)) => {
                if let Some(token) = apptoken.token() {
                    let auth = format!("token {}", token);
                    (parsed_url, Some(auth))
                } else {
                    unreachable!("request() should be handling the refresh!");
                }
            }
            None => (parsed_url, None),
        };

        Ok(tuple)
    }

    async fn request<Out>(
        &self,
        method: Method,
        uri: &str,
        body: Option<Vec<u8>>,
        media_type: MediaType,
        authentication: AuthenticationConstraint,
    ) -> Result<(Option<Link>, Out)>
    where
        Out: DeserializeOwned + 'static + Send + Sync,
    {
        if let Some(&Credentials::InstallationToken(ref apptoken)) =
            self.credentials(authentication)
        {
            if apptoken.token().is_none() {
                debug!("App token is stale, refreshing");
                let refresh_uri = self.host.clone()
                    + &format!(
                        "/app/installations/{}/access_tokens",
                        apptoken.installation_id
                    );
                let url_and_auth = self
                    .url_and_auth(&refresh_uri, AuthenticationConstraint::JWT)
                    .await?;

                let token_ref = apptoken.access_key.clone();

                let token_response: app::AccessToken = self
                    .authenticated_request(
                        Method::POST,
                        &refresh_uri,
                        url_and_auth.0,
                        url_and_auth.1,
                        Some(Vec::new()),
                        MediaType::Preview("machine-man"),
                    )
                    .await?
                    .1;

                *token_ref.lock().unwrap() = Some(token_response.token);
            }
        }

        let url_and_auth = self.url_and_auth(&uri, authentication).await?;
        self.authenticated_request(
            method,
            &uri,
            url_and_auth.0,
            url_and_auth.1,
            body,
            media_type,
        )
        .await
    }

    async fn authenticated_request<Out>(
        &self,
        method: Method,
        _uri2: &str,
        url: Url,
        auth: Option<String>,
        body: Option<Vec<u8>>,
        media_type: MediaType,
    ) -> Result<(Option<Link>, Out)>
    where
        Out: DeserializeOwned + 'static + Send + Sync,
    {
        let instance = self.clone();
        let body2 = body.clone();
        let method2 = method.clone();

        let response = {
            #[cfg(not(feature = "httpcache"))]
            let mut req = instance.client.request(method2, url);

            #[cfg(feature = "httpcache")]
            let mut req = {
                let mut req = instance.client.request(method2.clone(), url.clone());
                if method2 == Method::GET {
                    if let Ok(etag) = instance.http_cache.lookup_etag(&_uri2) {
                        req = req.header(IF_NONE_MATCH, etag);
                    }
                }
                req
            };

            req = req.header(USER_AGENT, &*instance.agent);
            req = req.header(
                ACCEPT,
                &*format!("{}", qitem::<Mime>(From::from(media_type))),
            );

            if let Some(auth_str) = auth {
                req = req.header(AUTHORIZATION, &*auth_str);
            }

            trace!("Body: {:?}", &body2);
            if let Some(body) = body2 {
                req = req.body(Body::from(body));
            }
            debug!("Request: {:?}", &req);

            req.send().await.map_err(Error::from)
        }?;

        #[cfg(feature = "httpcache")]
        let instance2 = self.clone();

        #[cfg(feature = "httpcache")]
        let uri3 = url.to_string();

        #[cfg(not(feature = "httpcache"))]
        let (remaining, reset) = get_header_values(response.headers());
        #[cfg(feature = "httpcache")]
        let (remaining, reset, etag) = get_header_values(response.headers());

        let status = response.status();
        let link = response
            .headers()
            .get(LINK)
            .and_then(|l| l.to_str().ok())
            .and_then(|l| l.parse().ok());

        let response_body = response.bytes().await?;
        if status.is_success() {
            debug!(
                "response payload {}",
                String::from_utf8_lossy(&response_body)
            );
            #[cfg(feature = "httpcache")]
            {
                if let Some(etag) = etag {
                    let next_link = link.as_ref().and_then(|l| utils::next_link(&l));
                    if let Err(e) = instance2.http_cache.cache_response(
                        &uri3,
                        &response_body,
                        &etag,
                        &next_link,
                    ) {
                        // failing to cache isn't fatal, so just log & swallow the error
                        debug!("Failed to cache body & etag: {}", e);
                    }
                }
            }
            let parsed_response = if status == StatusCode::NO_CONTENT {
                serde_json::from_str("null")
            } else {
                serde_json::from_slice::<Out>(&response_body)
            };
            parsed_response
                .map(|out| (link, out))
                .map_err(|error| ErrorKind::Codec(error).into())
        } else if status == StatusCode::NOT_MODIFIED {
            // only supported case is when client provides if-none-match
            // header when cargo builds with --cfg feature="httpcache"
            #[cfg(feature = "httpcache")]
            {
                instance2
                    .http_cache
                    .lookup_body(&uri3)
                    .map_err(Error::from)
                    .and_then(|body| {
                        serde_json::from_str::<Out>(&body)
                            .map_err(Error::from)
                            .and_then(|out| {
                                let link = match link {
                                    Some(link) => Ok(Some(link)),
                                    None => instance2.http_cache.lookup_next_link(&uri3).map(
                                        |next_link| {
                                            next_link.map(|next| {
                                                let next = LinkValue::new(next)
                                                    .push_rel(RelationType::Next);
                                                Link::new(vec![next])
                                            })
                                        },
                                    ),
                                };
                                link.map(|link| (link, out))
                            })
                    })
            }
            #[cfg(not(feature = "httpcache"))]
            {
                unreachable!("this should not be reachable without the httpcache feature enabled")
            }
        } else {
            let error = match (remaining, reset) {
                (Some(remaining), Some(reset)) if remaining == 0 => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    ErrorKind::RateLimit {
                        reset: Duration::from_secs(u64::from(reset) - now),
                    }
                }
                _ => ErrorKind::Fault {
                    code: status,
                    error: serde_json::from_slice(&response_body)?,
                },
            };
            Err(error.into())
        }
    }

    async fn request_entity<D>(
        &self,
        method: Method,
        uri: &str,
        body: Option<Vec<u8>>,
        media_type: MediaType,
        authentication: AuthenticationConstraint,
    ) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        let response: Result<(_, D)> = self
            .request(method, uri, body, media_type, authentication)
            .await;
        return Ok(response?.1);
    }

    async fn get<D>(&self, uri: &str) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.get_media(uri, MediaType::Json).await
    }

    async fn get_media<D>(&self, uri: &str, media: MediaType) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.request_entity(
            Method::GET,
            &(self.host.clone() + uri),
            None,
            media,
            AuthenticationConstraint::Unconstrained,
        )
        .await
    }

    async fn get_stream<D>(&self, uri: &str) -> Stream<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        unfold::<_, D>(self.clone(), self.get_pages(uri).await, Box::new(|x| x)).await
    }

    async fn get_pages<D>(&self, uri: &str) -> Result<(Option<Link>, D)>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.request(
            Method::GET,
            &(self.host.clone() + uri),
            None,
            MediaType::Json,
            AuthenticationConstraint::Unconstrained,
        )
        .await
    }

    async fn delete(&self, uri: &str) -> Result<()> {
        self.request_entity::<()>(
            Method::DELETE,
            &(self.host.clone() + uri),
            None,
            MediaType::Json,
            AuthenticationConstraint::Unconstrained,
        )
        .await
        .or_else(|err| match err {
            Error(ErrorKind::Codec(_), _) => Ok(()),
            otherwise => Err(otherwise),
        })
    }

    async fn delete_message(&self, uri: &str, message: Vec<u8>) -> Result<()> {
        self.request_entity::<()>(
            Method::DELETE,
            &(self.host.clone() + uri),
            Some(message),
            MediaType::Json,
            AuthenticationConstraint::Unconstrained,
        )
        .await
        .or_else(|err| match err {
            Error(ErrorKind::Codec(_), _) => Ok(()),
            otherwise => Err(otherwise),
        })
    }

    async fn post<D>(&self, uri: &str, message: Vec<u8>) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.post_media(
            uri,
            message,
            MediaType::Json,
            AuthenticationConstraint::Unconstrained,
        )
        .await
    }

    async fn post_media<D>(
        &self,
        uri: &str,
        message: Vec<u8>,
        media: MediaType,
        authentication: AuthenticationConstraint,
    ) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.request_entity(
            Method::POST,
            &(self.host.clone() + uri),
            Some(message),
            media,
            authentication,
        )
        .await
    }

    async fn patch_no_response(&self, uri: &str, message: Vec<u8>) -> Result<()> {
        self.patch(uri, message).await.or_else(|err| match err {
            Error(ErrorKind::Codec(_), _) => Ok(()),
            err => Err(err),
        })
    }

    async fn patch_media<D>(&self, uri: &str, message: Vec<u8>, media: MediaType) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.request_entity(
            Method::PATCH,
            &(self.host.clone() + uri),
            Some(message),
            media,
            AuthenticationConstraint::Unconstrained,
        )
        .await
    }

    async fn patch<D>(&self, uri: &str, message: Vec<u8>) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.patch_media(uri, message, MediaType::Json).await
    }

    async fn put_no_response(&self, uri: &str, message: Vec<u8>) -> Result<()> {
        self.put(uri, message).await.or_else(|err| match err {
            Error(ErrorKind::Codec(_), _) => Ok(()),
            err => Err(err),
        })
    }

    async fn put<D>(&self, uri: &str, message: Vec<u8>) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.put_media(uri, message, MediaType::Json).await
    }

    async fn put_media<D>(&self, uri: &str, message: Vec<u8>, media: MediaType) -> Result<D>
    where
        D: DeserializeOwned + 'static + Send + Sync,
    {
        self.request_entity(
            Method::PUT,
            &(self.host.clone() + uri),
            Some(message),
            media,
            AuthenticationConstraint::Unconstrained,
        )
        .await
    }
}

#[cfg(not(feature = "httpcache"))]
type HeaderValues = (Option<u32>, Option<u32>);
#[cfg(feature = "httpcache")]
type HeaderValues = (Option<u32>, Option<u32>, Option<Vec<u8>>);

fn get_header_values(headers: &HeaderMap<HeaderValue>) -> HeaderValues {
    if let Some(value) = headers.get(X_GITHUB_REQUEST_ID) {
        debug!("x-github-request-id: {:?}", value)
    }
    if let Some(value) = headers.get(X_RATELIMIT_LIMIT) {
        debug!("x-rate-limit-limit: {:?}", value)
    }
    let remaining = headers
        .get(X_RATELIMIT_REMAINING)
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse::<u32>().ok());
    let reset = headers
        .get(X_RATELIMIT_RESET)
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse::<u32>().ok());
    if let Some(value) = remaining {
        debug!("x-rate-limit-remaining: {}", value)
    }
    if let Some(value) = reset {
        debug!("x-rate-limit-reset: {}", value)
    }
    let etag = headers.get(ETAG);
    if let Some(value) = etag {
        debug!("etag: {:?}", value)
    }

    #[cfg(feature = "httpcache")]
    {
        let etag = etag.map(|etag| etag.as_bytes().to_vec());
        (remaining, reset, etag)
    }
    #[cfg(not(feature = "httpcache"))]
    (remaining, reset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sort_direction() {
        let default: SortDirection = Default::default();
        assert_eq!(default, SortDirection::Asc)
    }

    #[test]
    #[cfg(not(feature = "httpcache"))]
    fn header_values() {
        let empty = HeaderMap::new();
        let actual = get_header_values(&empty);
        let expected = (None, None);
        assert_eq!(actual, expected);

        let mut all_valid = HeaderMap::new();
        all_valid.insert(X_RATELIMIT_REMAINING, HeaderValue::from_static("1234"));
        all_valid.insert(X_RATELIMIT_RESET, HeaderValue::from_static("5678"));
        let actual = get_header_values(&all_valid);
        let expected = (Some(1234), Some(5678));
        assert_eq!(actual, expected);

        let mut invalid = HeaderMap::new();
        invalid.insert(X_RATELIMIT_REMAINING, HeaderValue::from_static("foo"));
        invalid.insert(X_RATELIMIT_RESET, HeaderValue::from_static("bar"));
        let actual = get_header_values(&invalid);
        let expected = (None, None);
        assert_eq!(actual, expected);
    }

    #[test]
    #[cfg(feature = "httpcache")]
    fn header_values() {
        let empty = HeaderMap::new();
        let actual = get_header_values(&empty);
        let expected = (None, None, None);
        assert_eq!(actual, expected);

        let mut all_valid = HeaderMap::new();
        all_valid.insert(X_RATELIMIT_REMAINING, HeaderValue::from_static("1234"));
        all_valid.insert(X_RATELIMIT_RESET, HeaderValue::from_static("5678"));
        all_valid.insert(ETAG, HeaderValue::from_static("foobar"));
        let actual = get_header_values(&all_valid);
        let expected = (Some(1234), Some(5678), Some(b"foobar".to_vec()));
        assert_eq!(actual, expected);

        let mut invalid = HeaderMap::new();
        invalid.insert(X_RATELIMIT_REMAINING, HeaderValue::from_static("foo"));
        invalid.insert(X_RATELIMIT_RESET, HeaderValue::from_static("bar"));
        invalid.insert(ETAG, HeaderValue::from_static(""));
        let actual = get_header_values(&invalid);
        let expected = (None, None, Some(Vec::new()));
        assert_eq!(actual, expected);
    }
}
