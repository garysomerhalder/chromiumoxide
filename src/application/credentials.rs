//! WebAuthn credentials (Playwright's `context.credentials`), backed by
//! Chromium's virtual authenticator (`WebAuthn` domain) on every page.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::page::{DEFAULT_TIMEOUT, Page};
use crate::domain::error::Result;
use crate::domain::frame::World;
use crate::ports::CdpSession;

/// A stored passkey. Binary fields are base64url without padding, like Playwright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub id: String,
    pub rp_id: String,
    pub user_handle: String,
    /// PKCS#8 private key.
    pub private_key: String,
    /// SPKI public key, when known.
    pub public_key: Option<String>,
}

/// Options for `credentials.create()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewCredential {
    pub rp_id: String,
    pub id: Option<String>,
    pub user_handle: Option<String>,
    pub private_key: Option<String>,
    pub public_key: Option<String>,
}

impl NewCredential {
    pub fn new(rp_id: impl Into<String>) -> Self {
        NewCredential {
            rp_id: rp_id.into(),
            ..Default::default()
        }
    }
}

#[derive(Default)]
pub(crate) struct CredentialState {
    installed: bool,
    registry: Vec<Credential>,
    /// Page target id -> virtual authenticator id.
    authenticators: HashMap<String, String>,
}

fn url_to_std(s: &str) -> String {
    URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .map(|b| STANDARD.encode(b))
        .unwrap_or_default()
}

fn std_to_url(s: &str) -> String {
    STANDARD
        .decode(s)
        .map(|b| URL_SAFE_NO_PAD.encode(b))
        .unwrap_or_default()
}

fn random_url(bytes: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    // Identifiers for test credentials: std's randomly seeded hasher is enough.
    let mut out = Vec::with_capacity(bytes);
    while out.len() < bytes {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(out.len());
        out.extend_from_slice(&h.finish().to_le_bytes());
    }
    out.truncate(bytes);
    URL_SAFE_NO_PAD.encode(out)
}

fn add_params(authenticator: &str, c: &Credential) -> Value {
    json!({
        "authenticatorId": authenticator,
        "credential": {
            "credentialId": url_to_std(&c.id),
            "isResidentCredential": true,
            "rpId": c.rp_id,
            "privateKey": url_to_std(&c.private_key),
            "userHandle": url_to_std(&c.user_handle),
            "signCount": 0,
        }
    })
}

/// Sets up the virtual authenticator of a page (before it runs).
pub(crate) async fn setup_page(
    cdp: &Arc<dyn CdpSession>,
    session_id: &str,
    target_id: &str,
    context: &ContextInner,
) -> Result<()> {
    let registry = {
        let st = context.credentials.lock().unwrap();
        if !st.installed {
            return Ok(());
        }
        st.registry.clone()
    };
    let sid = Some(session_id);
    cdp.call(sid, "WebAuthn.enable", json!({"enableUI": false}))
        .await?;
    let r = cdp
        .call(
            sid,
            "WebAuthn.addVirtualAuthenticator",
            json!({"options": {
                "protocol": "ctap2", "transport": "internal", "hasResidentKey": true,
                "hasUserVerification": true, "isUserVerified": true,
                "automaticPresenceSimulation": true,
            }}),
        )
        .await?;
    let id = r
        .get("authenticatorId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    for c in &registry {
        cdp.call(sid, "WebAuthn.addCredential", add_params(&id, c))
            .await?;
    }
    context
        .credentials
        .lock()
        .unwrap()
        .authenticators
        .insert(target_id.to_owned(), id);
    Ok(())
}

/// `context.credentials`: WebAuthn credentials of the context.
#[derive(Clone)]
pub struct Credentials {
    context: Arc<ContextInner>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").finish()
    }
}

impl BrowserContext {
    pub fn credentials(&self) -> Credentials {
        Credentials {
            context: self.inner.clone(),
        }
    }
}

impl Credentials {
    fn ctx(&self) -> BrowserContext {
        BrowserContext {
            inner: self.context.clone(),
        }
    }

    fn live_pages(&self) -> Vec<(Page, Option<String>)> {
        let st = self.context.credentials.lock().unwrap();
        self.ctx()
            .pages()
            .into_iter()
            .filter(|p| !p.is_closed())
            .map(|p| {
                let a = st.authenticators.get(p.target_id()).cloned();
                (p, a)
            })
            .collect()
    }

    /// Installs a virtual authenticator on every page (now and later).
    pub async fn install(&self) -> Result<()> {
        {
            let mut st = self.context.credentials.lock().unwrap();
            if st.installed {
                return Ok(());
            }
            st.installed = true;
        }
        for (page, _) in self.live_pages() {
            setup_page(
                &page.inner.shared.cdp,
                &page.inner.session_id,
                page.target_id(),
                &self.context,
            )
            .await?;
        }
        Ok(())
    }

    /// Creates a P-256 key with the browser's WebCrypto (as PKCS#8 and SPKI),
    /// in a helper page on a routed `https:` origin, which is a secure context.
    async fn generate_key(&self) -> Result<(String, String)> {
        let page = self.ctx().new_page().await?;
        let run = async {
            page.route("**/*", |r: crate::application::route::Route| async move {
                let _ = r.fulfill().body("<html></html>").await;
            })
            .await?;
            page.goto("https://chromiumoxide.invalid/").await?;
            page.call_function(
                World::Utility,
                "async () => { const k = await crypto.subtle.generateKey({name: 'ECDSA', namedCurve: 'P-256'}, true, ['sign']); \
                 const b = async (f, key) => btoa(String.fromCharCode(...new Uint8Array(await crypto.subtle.exportKey(f, key)))); \
                 return [await b('pkcs8', k.privateKey), await b('spki', k.publicKey)]; }",
                &[],
                DEFAULT_TIMEOUT,
                "generating a key",
            )
            .await
        };
        let r = run.await;
        let _ = page.close().await;
        let pair: (String, String) = serde_json::from_value(r?)?;
        Ok((std_to_url(&pair.0), std_to_url(&pair.1)))
    }

    /// `credentials.create(options)`: adds a passkey for `rp_id`.
    pub async fn create(&self, options: NewCredential) -> Result<Credential> {
        let (private_key, public_key) = match (options.private_key, options.public_key) {
            (Some(k), p) => (k, p),
            (None, _) => {
                let (k, p) = self.generate_key().await?;
                (k, Some(p))
            }
        };
        let credential = Credential {
            id: options.id.unwrap_or_else(|| random_url(16)),
            rp_id: options.rp_id,
            user_handle: options.user_handle.unwrap_or_else(|| random_url(16)),
            private_key,
            public_key,
        };
        self.context
            .credentials
            .lock()
            .unwrap()
            .registry
            .push(credential.clone());
        for (page, authenticator) in self.live_pages() {
            if let Some(a) = authenticator {
                page.send("WebAuthn.addCredential", add_params(&a, &credential))
                    .await?;
            }
        }
        Ok(credential)
    }

    /// Picks up credentials pages created through `navigator.credentials`.
    async fn sync(&self) -> Result<()> {
        for (page, authenticator) in self.live_pages() {
            let Some(a) = authenticator else { continue };
            let r = page
                .send("WebAuthn.getCredentials", json!({"authenticatorId": a}))
                .await?;
            for c in r
                .get("credentials")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default();
                let id = std_to_url(s("credentialId"));
                let mut st = self.context.credentials.lock().unwrap();
                if !st.registry.iter().any(|r| r.id == id) {
                    st.registry.push(Credential {
                        id,
                        rp_id: s("rpId").to_owned(),
                        user_handle: std_to_url(s("userHandle")),
                        private_key: std_to_url(s("privateKey")),
                        public_key: None,
                    });
                }
            }
        }
        Ok(())
    }

    /// `credentials.get({rpId, id})`.
    pub async fn get(&self, rp_id: Option<&str>, id: Option<&str>) -> Result<Vec<Credential>> {
        self.sync().await?;
        let st = self.context.credentials.lock().unwrap();
        Ok(st
            .registry
            .iter()
            .filter(|c| rp_id.is_none_or(|r| c.rp_id == r) && id.is_none_or(|i| c.id == i))
            .cloned()
            .collect())
    }

    /// `credentials.delete(id)`.
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.sync().await?;
        self.context
            .credentials
            .lock()
            .unwrap()
            .registry
            .retain(|c| c.id != id);
        for (page, authenticator) in self.live_pages() {
            if let Some(a) = authenticator {
                let _ = page
                    .send(
                        "WebAuthn.removeCredential",
                        json!({"authenticatorId": a, "credentialId": url_to_std(id)}),
                    )
                    .await;
            }
        }
        Ok(())
    }
}

pub(crate) type CredentialsState = Mutex<CredentialState>;
