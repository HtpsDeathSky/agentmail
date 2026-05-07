# Gmail OAuth Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the Gmail OAuth review gaps by replacing prompt-based login with loopback completion, refreshing tokens before Gmail mail operations, and making demo OAuth state checks meaningful.

**Architecture:** Keep the existing Gmail OAuth, XOAUTH2, and account storage work. Add one backend-managed loopback completion command, route all Gmail connection-settings preparation through an async refresh guard, and update the frontend/demo API to wait for backend completion instead of asking the user to paste code/state.

**Tech Stack:** Rust workspace (`app-api`, `mail-core`, `mail-store`, `mail-protocol`, `src-tauri`), Tokio, Tauri v2, SQLite, React/Vite TypeScript, Vitest, Cargo tests, pnpm.

---

## File Structure

- Modify `Cargo.toml`: enable Tokio `net` and `io-util` features for the local TCP listener.
- Modify `crates/app-api/src/lib.rs`: add loopback callback completion, async Gmail token refresh before connection settings, and backend tests.
- Modify `src-tauri/src/main.rs`: register the loopback completion command and keep the existing manual completion command until a later cleanup confirms it is unused.
- Modify `ui/src/api.ts`: add the loopback wait command type and API binding.
- Modify `ui/src/App.tsx`: remove `window.prompt` from Gmail sign-in and wait for backend completion.
- Modify `ui/src/data/demoBackend.ts`: store generated OAuth state per verifier and reject mismatches.
- Modify `ui/src/api.test.ts` and `ui/src/App.test.ts`: cover demo state mismatch and non-prompt Gmail sign-in behavior.
- Keep `docs/superpowers/specs/2026-05-07-gmail-oauth-review-fixes-design.md` as the design source of truth.

---

### Task 1: Backend Loopback OAuth Completion

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/app-api/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `crates/app-api/src/lib.rs`

- [ ] **Step 1: Write failing backend tests for callback parsing and state mismatch**

In `crates/app-api/src/lib.rs`, inside `mod tests`, add tests that prove the backend can parse a loopback callback URL and reject a wrong state before token exchange.

```rust
#[test]
fn parses_google_oauth_loopback_callback_url() {
    let callback = parse_google_oauth_callback_url(
        "http://127.0.0.1:53682/oauth/google/callback?code=auth%2Dcode&state=state%2Dvalue",
    )
    .unwrap();

    assert_eq!(callback.code, "auth-code");
    assert_eq!(callback.state, "state-value");
    assert_eq!(callback.error, None);
}

#[tokio::test]
async fn google_oauth_loopback_completion_rejects_state_mismatch() {
    let api = AppApi::new_with_google_oauth_client_id_for_tests(
        MailStore::memory().unwrap(),
        Arc::new(MockMailProtocol),
        Some("test-client-id.apps.googleusercontent.com".to_string()),
    );

    let start = api
        .start_google_oauth(GmailOAuthStartRequest {
            email: "user@gmail.com".to_string(),
            display_name: "User".to_string(),
        })
        .unwrap();

    let error = api
        .complete_google_oauth_from_loopback(GmailOAuthLoopbackCompleteRequest {
            verifier_id: start.verifier_id,
            callback_url:
                "http://127.0.0.1:53682/oauth/google/callback?code=auth-code&state=wrong-state"
                    .to_string(),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(error, ApiError::InvalidRequest(message) if message.contains("state mismatch"))
    );
}
```

Run:

```bash
cargo test -p app-api parses_google_oauth_loopback_callback_url google_oauth_loopback_completion_rejects_state_mismatch
```

Expected: FAIL because `parse_google_oauth_callback_url`, `GmailOAuthLoopbackCompleteRequest`, and `complete_google_oauth_from_loopback` do not exist.

- [ ] **Step 2: Add loopback completion request type**

In `crates/app-api/src/lib.rs`, near the existing Gmail OAuth DTOs, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailOAuthLoopbackCompleteRequest {
    pub verifier_id: String,
    pub callback_url: String,
}
```

Also update the `use app_api::{ ... }` list in `src-tauri/src/main.rs` to include `GmailOAuthLoopbackCompleteRequest`.

- [ ] **Step 3: Add callback query parsing helpers**

In `crates/app-api/src/lib.rs`, near the Google OAuth helper functions, add a small parser that accepts only the expected loopback callback URL shape:

```rust
#[derive(Debug, PartialEq, Eq)]
struct GoogleOAuthCallback {
    code: String,
    state: String,
    error: Option<String>,
}

fn parse_google_oauth_callback_url(value: &str) -> ApiResult<GoogleOAuthCallback> {
    let (_, query) = value
        .split_once('?')
        .ok_or_else(|| ApiError::InvalidRequest("oauth callback query is missing".to_string()))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;

    for pair in query.split('&') {
        let (key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "code" => code = Some(form_decode(raw_value)?),
            "state" => state = Some(form_decode(raw_value)?),
            "error" => error = Some(form_decode(raw_value)?),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Ok(GoogleOAuthCallback {
            code: String::new(),
            state: state.unwrap_or_default(),
            error: Some(error),
        });
    }

    Ok(GoogleOAuthCallback {
        code: code
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::InvalidRequest("oauth callback code is missing".to_string()))?,
        state: state
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::InvalidRequest("oauth callback state is missing".to_string()))?,
        error: None,
    })
}

fn form_decode(value: &str) -> ApiResult<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.as_bytes().iter().copied();
    while let Some(byte) = chars.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hi = chars
                    .next()
                    .ok_or_else(|| ApiError::InvalidRequest("invalid percent escape".to_string()))?;
                let lo = chars
                    .next()
                    .ok_or_else(|| ApiError::InvalidRequest("invalid percent escape".to_string()))?;
                bytes.push((hex_value(hi)? << 4) | hex_value(lo)?);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| ApiError::InvalidRequest("callback query is not utf8".to_string()))
}

fn hex_value(byte: u8) -> ApiResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ApiError::InvalidRequest("invalid percent escape".to_string())),
    }
}
```

- [ ] **Step 4: Implement loopback completion without starting the listener yet**

In `impl AppApi`, add:

```rust
pub async fn complete_google_oauth_from_loopback(
    &self,
    request: GmailOAuthLoopbackCompleteRequest,
) -> ApiResult<MailAccount> {
    let callback = parse_google_oauth_callback_url(&request.callback_url)?;
    if let Some(error) = callback.error {
        self.google_oauth_sessions.lock().remove(&request.verifier_id);
        return Err(ApiError::InvalidRequest(format!("google oauth error: {error}")));
    }
    self.complete_google_oauth(GmailOAuthCompleteRequest {
        verifier_id: request.verifier_id,
        authorization_code: callback.code,
        state: callback.state,
    })
    .await
}
```

Register a Tauri command in `src-tauri/src/main.rs`:

```rust
#[tauri::command]
async fn complete_google_oauth_from_loopback(
    state: State<'_, ApiState>,
    request: GmailOAuthLoopbackCompleteRequest,
) -> Result<MailAccount, String> {
    state
        .api
        .complete_google_oauth_from_loopback(request)
        .await
        .map_err(to_error)
}
```

Add it to `tauri::generate_handler![...]`.

- [ ] **Step 5: Run the parser and state validation tests**

Run:

```bash
cargo test -p app-api parses_google_oauth_loopback_callback_url google_oauth_loopback_completion_rejects_state_mismatch
```

Expected: PASS. These tests must not call the Google token endpoint.

- [ ] **Step 6: Add the actual loopback wait command**

Enable Tokio net/io features in root `Cargo.toml`:

```toml
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "net", "io-util"] }
```

In `crates/app-api/src/lib.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailOAuthWaitForCallbackRequest {
    pub verifier_id: String,
}
```

Add an async method:

```rust
pub async fn wait_for_google_oauth_callback(
    &self,
    request: GmailOAuthWaitForCallbackRequest,
) -> ApiResult<MailAccount> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:53682")
        .await
        .map_err(|error| ApiError::InvalidRequest(format!("oauth callback listener failed: {error}")))?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(180), listener.accept())
        .await
        .map_err(|_| ApiError::InvalidRequest("google oauth timed out waiting for callback".to_string()))?
        .map_err(|error| ApiError::InvalidRequest(format!("oauth callback accept failed: {error}")))?;
    let (mut stream, _) = result;
    let mut buffer = vec![0_u8; 8192];
    let bytes = tokio::io::AsyncReadExt::read(&mut stream, &mut buffer)
        .await
        .map_err(|error| ApiError::InvalidRequest(format!("oauth callback read failed: {error}")))?;
    let request_line = std::str::from_utf8(&buffer[..bytes])
        .ok()
        .and_then(|request| request.lines().next())
        .ok_or_else(|| ApiError::InvalidRequest("oauth callback request is invalid".to_string()))?;
    let path = request_line
        .strip_prefix("GET ")
        .and_then(|rest| rest.split_once(' ').map(|(path, _)| path.to_string()))
        .ok_or_else(|| ApiError::InvalidRequest("oauth callback request line is invalid".to_string()))?;
    let callback_url = format!("http://127.0.0.1:53682{path}");
    let account = self
        .complete_google_oauth_from_loopback(GmailOAuthLoopbackCompleteRequest {
            verifier_id: request.verifier_id,
            callback_url,
        })
        .await;
    let body = if account.is_ok() {
        "AgentMail Google sign-in complete. You can close this tab."
    } else {
        "AgentMail Google sign-in failed. Return to the app and try again."
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
    account
}
```

Register the new Tauri command:

```rust
#[tauri::command]
async fn wait_for_google_oauth_callback(
    state: State<'_, ApiState>,
    request: GmailOAuthWaitForCallbackRequest,
) -> Result<MailAccount, String> {
    state
        .api
        .wait_for_google_oauth_callback(request)
        .await
        .map_err(to_error)
}
```

- [ ] **Step 7: Commit**

Run:

```bash
cargo fmt --all
cargo test -p app-api parses_google_oauth_loopback_callback_url google_oauth_loopback_completion_rejects_state_mismatch
git add Cargo.toml crates/app-api/src/lib.rs src-tauri/src/main.rs
git commit -m "fix: complete gmail oauth through loopback"
```

Expected: formatting succeeds and the targeted tests pass. Do not commit if Rust compilation fails.

---

### Task 2: Automatic Gmail Token Refresh Before Mail Operations

**Files:**
- Modify: `crates/app-api/src/lib.rs`
- Test: `crates/app-api/src/lib.rs`

- [ ] **Step 1: Write a failing test for expired token refresh**

In `crates/app-api/src/lib.rs`, inside `mod tests`, add a test around a test-only injectable token refresher. If no injectable refresher exists yet, this test should fail to compile first.

```rust
#[tokio::test]
async fn expired_gmail_token_refreshes_before_connection_settings() {
    let api = AppApi::new_with_google_oauth_client_id_for_tests(
        MailStore::memory().unwrap(),
        Arc::new(MockMailProtocol),
        Some("test-client-id.apps.googleusercontent.com".to_string()),
    )
    .with_google_token_refresher_for_tests(Arc::new(|refresh_token| {
        assert_eq!(refresh_token, "refresh-token");
        Ok(GoogleOAuthTokenResponse {
            access_token: "fresh-access-token".to_string(),
            expires_in: 3600,
            refresh_token: None,
        })
    }));

    let account = MailAccount {
        id: "gmail-acct".to_string(),
        display_name: "Gmail".to_string(),
        email: "user@gmail.com".to_string(),
        provider: MailProvider::Gmail,
        auth: MailAuth::GoogleOAuth {
            refresh_token: "refresh-token".to_string(),
            access_token: "expired-access-token".to_string(),
            expires_at: "2000-01-01T00:00:00Z".to_string(),
        },
        imap_host: "imap.gmail.com".to_string(),
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.gmail.com".to_string(),
        smtp_port: 465,
        smtp_tls: true,
        sync_enabled: true,
        created_at: "2026-05-07T00:00:00Z".to_string(),
        updated_at: "2026-05-07T00:00:00Z".to_string(),
    };
    api.store.save_account(&account).unwrap();

    let settings = api.connection_settings_for_account(&account).await.unwrap();

    assert_eq!(
        settings.auth,
        ConnectionAuth::GoogleOAuth {
            access_token: "fresh-access-token".to_string(),
        }
    );
    let loaded = api.store.get_account("gmail-acct").unwrap();
    assert!(matches!(
        loaded.auth,
        MailAuth::GoogleOAuth { access_token, .. } if access_token == "fresh-access-token"
    ));
}
```

Run:

```bash
cargo test -p app-api expired_gmail_token_refreshes_before_connection_settings
```

Expected: FAIL because token refresher injection and async connection settings do not exist yet.

- [ ] **Step 2: Add a token refresher seam**

In `crates/app-api/src/lib.rs`, add a small internal type near `GoogleOAuthConfig`:

```rust
type GoogleTokenRefreshFn =
    Arc<dyn Fn(String) -> ApiResult<GoogleOAuthTokenResponse> + Send + Sync>;
```

Add this field to `AppApi`:

```rust
google_token_refresher: Option<GoogleTokenRefreshFn>,
```

Initialize it as `None` in constructors. Add a test-only builder:

```rust
#[cfg(test)]
fn with_google_token_refresher_for_tests(mut self, refresher: GoogleTokenRefreshFn) -> Self {
    self.google_token_refresher = Some(refresher);
    self
}
```

- [ ] **Step 3: Convert connection settings helper to async**

Change the helper signature from:

```rust
fn connection_settings_for_account(
    &self,
    account: &MailAccount,
) -> ApiResult<ConnectionSettings> {
    account_to_settings(account)
}
```

to:

```rust
async fn connection_settings_for_account(
    &self,
    account: &MailAccount,
) -> ApiResult<ConnectionSettings> {
    let account = self.refresh_google_oauth_if_needed(account).await?;
    account_to_settings(&account)
}
```

Add:

```rust
async fn refresh_google_oauth_if_needed(&self, account: &MailAccount) -> ApiResult<MailAccount> {
    let MailAuth::GoogleOAuth {
        refresh_token,
        access_token,
        expires_at,
    } = &account.auth
    else {
        return Ok(account.clone());
    };

    if !gmail_access_token_needs_refresh(access_token, expires_at) {
        return Ok(account.clone());
    }
    if refresh_token.is_empty() {
        return Err(ApiError::InvalidRequest(
            "google sign-in expired; sign in again".to_string(),
        ));
    }

    let token = if let Some(refresher) = &self.google_token_refresher {
        refresher(refresh_token.clone())?
    } else {
        let client_id = self.google_oauth_client_id()?;
        refresh_google_oauth_token(&client_id, refresh_token).await?
    };

    let mut refreshed = account.clone();
    refreshed.auth = MailAuth::GoogleOAuth {
        refresh_token: refresh_token.clone(),
        access_token: token.access_token,
        expires_at: now_plus_seconds_rfc3339(token.expires_in),
    };
    refreshed.updated_at = now_rfc3339();
    self.store.save_account(&refreshed)?;
    Ok(refreshed)
}
```

Add a helper that treats empty token, empty timestamp, invalid timestamp, or past timestamp as refresh-required:

```rust
fn gmail_access_token_needs_refresh(access_token: &str, expires_at: &str) -> bool {
    if access_token.is_empty() || expires_at.is_empty() {
        return true;
    }
    !timestamp_is_future(expires_at)
}
```

- [ ] **Step 4: Update all call sites to await the helper**

In `crates/app-api/src/lib.rs`, update all current `self.connection_settings_for_account(...)` call sites. The affected paths are:

- `test_account_connection` account-id path.
- `sync_folder`.
- `sync_account_inner`.
- `watch_folder_until_change`.
- `send_message_now`.
- `reconcile_sent_placeholders_after_send`.
- `execute_confirmed_mail_action`.
- `resolve_permanent_delete_uids`.

Each call should become:

```rust
let settings = self.connection_settings_for_account(&account).await?;
```

For methods that are currently sync and need the helper, either keep them sync only if they do not call the helper, or convert them to async and update callers. Do not use `block_on`.

- [ ] **Step 5: Add a fresh-token no-refresh test**

Add:

```rust
#[tokio::test]
async fn fresh_gmail_token_reuses_existing_access_token() {
    let api = AppApi::new_with_google_oauth_client_id_for_tests(
        MailStore::memory().unwrap(),
        Arc::new(MockMailProtocol),
        Some("test-client-id.apps.googleusercontent.com".to_string()),
    )
    .with_google_token_refresher_for_tests(Arc::new(|_| {
        panic!("fresh token should not refresh");
    }));

    let account = gmail_oauth_account(
        "User".to_string(),
        "user@gmail.com".to_string(),
        "refresh-token".to_string(),
        "current-access-token".to_string(),
        3600,
    );
    api.store.save_account(&account).unwrap();

    let settings = api.connection_settings_for_account(&account).await.unwrap();

    assert_eq!(
        settings.auth,
        ConnectionAuth::GoogleOAuth {
            access_token: "current-access-token".to_string(),
        }
    );
}
```

- [ ] **Step 6: Run targeted tests**

Run:

```bash
cargo test -p app-api expired_gmail_token_refreshes_before_connection_settings fresh_gmail_token_reuses_existing_access_token gmail_oauth_accounts_use_xoauth2_connection_settings
```

Expected: all three pass. If `gmail_oauth_accounts_use_xoauth2_connection_settings` now fails to compile because the helper is async, update it to `#[tokio::test]` and add `.await`.

- [ ] **Step 7: Commit**

Run:

```bash
cargo fmt --all
cargo test -p app-api expired_gmail_token_refreshes_before_connection_settings fresh_gmail_token_reuses_existing_access_token
git add crates/app-api/src/lib.rs
git commit -m "fix: refresh gmail tokens before mail operations"
```

Expected: targeted tests pass and the commit includes only app-api changes for token refresh.

---

### Task 3: Frontend Gmail Sign-In Without Prompts

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/App.test.ts`
- Test: `ui/src/App.test.ts`

- [ ] **Step 1: Add API types and a failing UI helper test**

In `ui/src/api.ts`, add:

```ts
export interface GmailOAuthWaitForCallbackRequest {
  verifier_id: string;
}
```

Add `wait_for_google_oauth_callback: MailAccount;` to `CommandMap`, and add:

```ts
waitForGoogleOAuthCallback: (request: GmailOAuthWaitForCallbackRequest) =>
  call("wait_for_google_oauth_callback", { request }),
```

In `ui/src/App.test.ts`, add a pure helper test for a function that does not exist yet:

```ts
import { runGoogleSignInFlow } from "./App";

describe("runGoogleSignInFlow", () => {
  it("opens google authorization and waits for backend callback without prompting", async () => {
    const prompt = vi.spyOn(window, "prompt").mockReturnValue("manual-code");
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const account = {
      id: "gmail-account",
      display_name: "Gmail",
      email: "user@gmail.com",
      provider: "gmail" as const,
      imap_host: "imap.gmail.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.gmail.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true,
      created_at: "2026-05-07T00:00:00Z",
      updated_at: "2026-05-07T00:00:00Z"
    };

    const result = await runGoogleSignInFlow({
      email: "user@gmail.com",
      displayName: "Gmail",
      startGoogleOAuth: vi.fn().mockResolvedValue({
        authorization_url: "https://accounts.google.com/o/oauth2/v2/auth",
        verifier_id: "verifier",
        redirect_uri: "http://127.0.0.1:53682/oauth/google/callback"
      }),
      waitForGoogleOAuthCallback: vi.fn().mockResolvedValue(account),
      openAuthorizationUrl: (url) => window.open(url, "_blank", "noopener,noreferrer")
    });

    expect(result).toEqual(account);
    expect(open).toHaveBeenCalledWith(
      "https://accounts.google.com/o/oauth2/v2/auth",
      "_blank",
      "noopener,noreferrer"
    );
    expect(prompt).not.toHaveBeenCalled();
    prompt.mockRestore();
    open.mockRestore();
  });
});
```

Run:

```bash
pnpm test -- App.test.ts
```

Expected: FAIL because `runGoogleSignInFlow` does not exist.

- [ ] **Step 2: Implement the pure sign-in helper**

In `ui/src/App.tsx`, export:

```ts
export interface GoogleSignInFlowDeps {
  email: string;
  displayName: string;
  startGoogleOAuth: typeof api.startGoogleOAuth;
  waitForGoogleOAuthCallback: typeof api.waitForGoogleOAuthCallback;
  openAuthorizationUrl: (url: string) => void;
}

export async function runGoogleSignInFlow({
  email,
  displayName,
  startGoogleOAuth,
  waitForGoogleOAuthCallback,
  openAuthorizationUrl
}: GoogleSignInFlowDeps): Promise<MailAccount> {
  const start = await startGoogleOAuth({
    email,
    display_name: displayName || "Gmail"
  });
  openAuthorizationUrl(start.authorization_url);
  return waitForGoogleOAuthCallback({ verifier_id: start.verifier_id });
}
```

Make sure `MailAccount` is imported from `ui/src/api.ts` if not already available.

- [ ] **Step 3: Replace prompt-based Gmail UI logic**

In `ui/src/App.tsx`, change `signInWithGoogle` so it calls the helper:

```ts
const signInWithGoogle = async () => {
  setAccountBusy(true);
  setAccountStatus("opening browser for google sign in");
  try {
    const account = await runGoogleSignInFlow({
      email: accountForm.email,
      displayName: accountForm.display_name || "Gmail",
      startGoogleOAuth: api.startGoogleOAuth,
      waitForGoogleOAuthCallback: api.waitForGoogleOAuthCallback,
      openAuthorizationUrl: (url) => {
        if (typeof window !== "undefined") {
          window.open(url, "_blank", "noopener,noreferrer");
        }
      }
    });
    setAccountStatus("google authorization received");
    setSelectedId(account.id);
    setAccountProvider("gmail");
    await onAccountSaved(account);
    setAccountStatus(`google sign in complete: ${account.email}`);
  } catch (error) {
    setAccountStatus(`google sign in failed: ${String(error)}`);
  } finally {
    setAccountBusy(false);
  }
};
```

Do not leave `window.prompt` anywhere in the Gmail sign-in path.

- [ ] **Step 4: Run frontend targeted tests**

Run:

```bash
pnpm test -- App.test.ts
rg -n "Google authorization code|Google OAuth state|window\\.prompt" ui/src/App.tsx
```

Expected: tests pass, and `rg` returns no prompt-based Gmail OAuth strings.

- [ ] **Step 5: Commit**

Run:

```bash
git add ui/src/api.ts ui/src/App.tsx ui/src/App.test.ts
git commit -m "fix: wait for gmail oauth callback in ui"
```

Expected: commit contains only frontend API/UI/test changes.

---

### Task 4: Demo OAuth State Validation

**Files:**
- Modify: `ui/src/data/demoBackend.ts`
- Modify: `ui/src/api.test.ts`
- Test: `ui/src/api.test.ts`

- [ ] **Step 1: Write a failing demo state mismatch test**

In `ui/src/api.test.ts`, add:

```ts
it("rejects Gmail OAuth completion with the wrong demo state", async () => {
  const start = await api.startGoogleOAuth({
    email: "demo-state@gmail.com",
    display_name: "Demo State"
  });

  await expect(
    api.completeGoogleOAuth({
      verifier_id: start.verifier_id,
      authorization_code: "demo-code",
      state: "wrong-state"
    })
  ).rejects.toThrow(/state/i);
});
```

Run:

```bash
pnpm test -- api.test.ts
```

Expected: FAIL because the demo backend currently accepts wrong state.

- [ ] **Step 2: Store state per demo verifier**

In `ui/src/data/demoBackend.ts`, change the OAuth session storage shape from storing only the request to storing request plus state:

```ts
type DemoGmailOAuthSession = GmailOAuthStartRequest & { state: string };
const gmailOAuthSessions: Record<string, DemoGmailOAuthSession> = {};
```

In `start_google_oauth`, generate and store state:

```ts
const state = crypto.randomUUID();
gmailOAuthSessions[verifierId] = { ...request, state };
```

Use that state in `authorization_url`:

```ts
`...&state=${encodeURIComponent(state)}`
```

- [ ] **Step 3: Validate state on demo completion**

In `complete_google_oauth`, before creating the account:

```ts
if (request.state !== session.state) {
  delete gmailOAuthSessions[request.verifier_id];
  throw new Error("oauth state mismatch");
}
```

- [ ] **Step 4: Update the happy-path demo test to use the generated state**

In `ui/src/api.test.ts`, parse state from the returned authorization URL:

```ts
const state = new URL(start.authorization_url).searchParams.get("state");
expect(state).toBeTruthy();
const account = await api.completeGoogleOAuth({
  verifier_id: start.verifier_id,
  authorization_code: "demo-code",
  state: state ?? ""
});
```

- [ ] **Step 5: Run demo API tests**

Run:

```bash
pnpm test -- api.test.ts
```

Expected: Gmail OAuth happy path and mismatch tests pass.

- [ ] **Step 6: Commit**

Run:

```bash
git add ui/src/data/demoBackend.ts ui/src/api.test.ts
git commit -m "fix: validate demo gmail oauth state"
```

Expected: commit contains only demo/test changes.

---

### Task 5: Full Verification and Review Cleanup

**Files:**
- Modify only files required by fixes from Tasks 1-4.

- [ ] **Step 1: Run Rust formatting check**

Run:

```bash
cargo fmt --all --check
```

Expected: PASS. If it fails, run `cargo fmt --all`, inspect the diff, and continue.

- [ ] **Step 2: Run Rust tests**

Run:

```bash
cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api
```

Expected: PASS. If failures are unrelated to Gmail OAuth, record exact failures before deciding whether to fix.

- [ ] **Step 3: Run frontend tests**

Run:

```bash
pnpm test
```

Expected: PASS.

- [ ] **Step 4: Run frontend build**

Run:

```bash
pnpm build
```

Expected: PASS.

- [ ] **Step 5: Run diff hygiene checks**

Run:

```bash
git diff --check
git status --short
```

Expected: no whitespace issues. `git status --short` may still show pre-existing untracked `log.txt`, `scripts/`, and the older untracked Gmail plan if they were not intentionally tracked in this repair.

- [ ] **Step 6: Commit verification-only cleanup when verification changed tracked files**

Run this only when formatting or small test cleanup changed tracked files after prior commits:

```bash
git add Cargo.toml crates/app-api/src/lib.rs src-tauri/src/main.rs ui/src/api.ts ui/src/App.tsx ui/src/App.test.ts ui/src/data/demoBackend.ts ui/src/api.test.ts
git commit -m "chore: verify gmail oauth fixes"
```

Expected: no commit is created unless there are real cleanup changes.

---

## Plan Self-Review

- Spec coverage: OAuth loopback completion is covered by Task 1, token refresh by Task 2, UI prompt removal by Task 3, demo state validation by Task 4, and full verification by Task 5.
- Scope check: This plan does not add public Google verification work, new providers, or unrelated account settings changes.
- Type consistency: `GmailOAuthLoopbackCompleteRequest` and `GmailOAuthWaitForCallbackRequest` are named consistently across backend and frontend steps.
- Test strategy: Each task starts with a failing test or check and finishes with targeted verification before commit.
