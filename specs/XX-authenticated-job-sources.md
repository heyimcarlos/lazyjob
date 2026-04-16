# Spec: Authenticated Job Source Integration (LinkedIn/Indeed/Glassdoor)

## Context

Job discovery currently uses public APIs (Greenhouse, Lever). The highest-value sources (LinkedIn, Indeed, Glassdoor) require authentication. This spec addresses cookie/session-based authentication for job platforms.

## Motivation

- **Coverage gap**: LinkedIn has 20M+ job listings, most not accessible via public APIs
- **Authentication complexity**: Session management, cookie rotation, Captcha handling
- **Security risks**: Storing session credentials introduces attack surface

## Design

### Credential Storage

All credentials stored via OS keyring (see `16-privacy-security.md`):

```rust
pub struct PlatformCredentials {
    pub platform: Platform,
    pub credential_type: CredentialType,
    pub stored_at: DateTime<Utc>,
}

pub enum CredentialType {
    CookieJar(CookieJar),  // Browser cookies
    OAuthToken(String),     // OAuth access token
    UsernamePassword { user: String, pass: String },
}

pub enum Platform {
    LinkedIn,
    Indeed,
    Glassdoor,
}
```

### LinkedIn Authentication

#### Cookie-Based Session

```rust
pub struct LinkedInAuth {
    keyring: KeyringService,
}

impl LinkedInAuth {
    /// Import cookies from browser extension (e.g., "Get cookies.txt")
    pub fn import_cookies(&self, cookie_file: &Path) -> Result<CookieJar> {
        let cookies = Self::parse_netscape_cookies(cookie_file)?;
        
        // Validate cookie jar has essential cookies
        let has_li_at = cookies.iter().any(|c| c.name == "li_at");
        if !has_li_at {
            return Err(AuthError::MissingSessionToken);
        }
        
        // Test session validity
        if !self.test_session(&cookies).await? {
            return Err(AuthError::SessionExpired);
        }
        
        Ok(cookies)
    }
    
    async fn test_session(&self, cookies: &CookieJar) -> Result<bool> {
        let client = self.build_client(cookies);
        let resp = client.get("https://www.linkedin.com/voyager/api/me").send().await?;
        Ok(resp.status() == 200)
    }
}
```

#### Session Rotation

- Monitor session expiry by checking `/voyager/api/me` periodically
- When session expires: notify user, prompt for new cookie import
- **No automatic re-authentication**: User must manually provide new cookies (prevents credential guessing)

### Indeed Authentication

Indeed allows searching without login but hides salary data and some listings behind auth:

```rust
impl IndeedAuth {
    pub fn build_auth_client(&self, cookies: CookieJar) -> reqwest::Client {
        // Indeed session cookie: "夕"
        let jar = CookieJar::new();
        jar.add_cookie_str("sn=...", &self.url);
        // ...
    }
}
```

### Captcha Handling

When encountering Captcha:

1. **Detection**: HTTP 403 with Captcha challenge page
2. **Response**: Pause discovery for this source, notify user
3. **User action**: User must solve Captcha in browser, provide new cookies
4. **No auto-solving**: Captcha solving services are unreliable and may be against ToS

### Session Security

```rust
pub struct SecureCookieJar {
    jar: CookieJar,
    encryption_key: [u8; 32],  // Derived from master password
}

impl SecureCookieJar {
    /// Encrypt cookie jar at rest
    pub fn encrypt(&self) -> Vec<u8> { /* ... */ }
    
    /// Decrypt cookie jar for use
    pub fn decrypt(encrypted: &[u8], key: &[u8]) -> Result<Self> { /* ... */ }
}
```

- Cookie jars encrypted with same key as database
- Never stored in plaintext
- Memory-mapped only during active discovery

## Implementation Notes

- Use `reqwest` with cookie jar middleware
- Rotate user agent to avoid detection
- Add random delays between requests
- Rate limit: max 1 request/2 seconds per source

## Open Questions

1. **Browser extension integration**: Tools like "Import cookies.txt" make this easier
2. **ToS risk**: This may violate platform ToS. Document clearly.
3. **2FA accounts**: If LinkedIn has 2FA, cookies eventually expire

## Related Specs

- `16-privacy-security.md` - Keyring integration
- `agent-interfaces-job-platforms.md` - Platform classification
- `XX-browser-fingerprinting-evasion.md` - Evasion techniques