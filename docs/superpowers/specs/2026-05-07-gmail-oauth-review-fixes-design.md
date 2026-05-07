# Gmail OAuth Review Fixes Design

## Goal

Fix the three review blockers in the Gmail compatibility branch so Gmail sign-in is a real desktop OAuth flow, Gmail IMAP/SMTP operations refresh expired access tokens automatically, and browser demo tests exercise the same state checks as the backend.

## Scope

This design is limited to the current Gmail compatibility branch. It does not expand provider support, add public Google app verification work, or change the existing password-based IMAP/SMTP flow.

The fixes cover:

- Replacing the prompt-based Google authorization code entry with a backend-managed loopback callback.
- Refreshing Gmail OAuth access tokens before sync, send, watch, test, and remote mail actions.
- Updating the browser demo OAuth state handling so tests catch state mismatch regressions.

## Current Problems

The backend generates `http://127.0.0.1:53682/oauth/google/callback` as the redirect URI, but the frontend currently opens the Google URL and asks the user to paste `authorization_code` and `state` through `window.prompt`. No local HTTP listener receives the callback, so the advertised installed-app flow is not complete.

`refresh_google_oauth` exists, but normal mail paths call `connection_settings_for_account()` and reuse the stored `access_token` directly. Once the token expires, Gmail sync and send will fail until the user manually reauthorizes.

The browser demo hard-codes `demo-state` and accepts completion without validating the stored state. The demo test therefore proves only that the demo path returns an account, not that OAuth state protection works.

## Chosen Approach

Use a short-lived loopback HTTP listener owned by the backend for each Google sign-in attempt. The frontend starts the OAuth attempt, opens the returned authorization URL, then waits for a backend command to finish the callback exchange. The user never handles the authorization code manually.

Use a single backend helper that prepares connection settings for remote mail operations. For Gmail accounts it checks `expires_at`, refreshes when needed, persists the new token, and returns `ConnectionAuth::GoogleOAuth` with a fresh access token. Password accounts continue to use the current password auth path.

Keep demo behavior lightweight but stateful. The demo backend stores the generated state per verifier and rejects completion when the caller provides the wrong state. UI tests should cover the new non-prompt flow and state mismatch handling.

## Architecture

### Backend OAuth Completion

`AppApi::start_google_oauth` should continue to create `verifier_id`, `state`, PKCE verifier/challenge, and the Google authorization URL. It should also prepare a pending session that can be completed only once.

Add a new async backend command such as `wait_for_google_oauth_callback` or replace the current two-step manual completion with `complete_google_oauth_from_loopback`. The command starts or attaches to a loopback listener for the pending verifier, waits for `/oauth/google/callback?code=...&state=...`, validates state, exchanges the code, persists the Gmail account, and returns `MailAccount`.

The callback response shown in the browser should be a small plain HTML page saying the sign-in is complete and the browser tab can be closed. Error responses should be plain text or minimal HTML and must not expose tokens.

The listener should bind to `127.0.0.1` only. The initial implementation can keep port `53682` if it handles "port already in use" cleanly. If port binding fails, return a clear error instructing the user to retry after freeing the port. A later improvement can allocate dynamic ports, but this fix should stay scoped.

### Token Refresh

Add a helper in `AppApi`, for example `connection_settings_for_account_async(&self, account: &MailAccount) -> ApiResult<ConnectionSettings>`, or convert the current helper to async. For `MailAuth::GoogleOAuth`, it should:

- Parse `expires_at`.
- Treat missing, invalid, or near-expired timestamps as refresh-required.
- Refresh with the stored refresh token before returning settings.
- Persist the new `access_token`, `expires_at`, and `updated_at`.
- Return a reauthorization error if refresh token is missing, revoked, or rejected by Google.

All remote protocol paths must use this helper: account connection testing by account ID, full sync, single-folder sync, IDLE watch, send, Sent reconciliation after send, confirmed remote actions, and permanent-delete UID resolution.

Generic IMAP/SMTP accounts should remain synchronous in behavior from the user point of view: saved password accounts must keep working unchanged.

### Demo and UI

The frontend should remove `window.prompt` for Google sign-in. The Gmail button should:

1. Call `startGoogleOAuth`.
2. Open `authorization_url`.
3. Set status to waiting for Google callback.
4. Await backend callback completion.
5. Save/select the returned account and refresh account state.

For the browser demo, `start_google_oauth` should generate and store a random state for the verifier. `complete_google_oauth` should compare request state to stored state and reject mismatch. Because the browser demo has no real Google callback, it may still simulate completion, but the simulation must use the stored state instead of a fixed state.

## Error Handling

If the user closes the Google tab or never completes authorization, the wait command should time out and leave the UI in a retryable state. The pending verifier should be cleaned up after timeout or completion.

If Google returns `error`, the backend should surface a concise message that includes the error code but not token material.

If token refresh fails during mail operations, the API should return a clear "Google sign-in expired; sign in again" style error. It should not silently fall back to password auth or mark the account as generic.

State mismatch should be treated as invalid request and should remove the pending verifier so the same verifier cannot be replayed.

## Testing

Backend tests should cover:

- Start URL includes state, PKCE challenge, offline access, and Gmail scope.
- Loopback completion validates state before token exchange.
- State mismatch rejects and consumes the pending session.
- Expired Gmail access token triggers refresh before returning connection settings.
- Fresh Gmail access token is reused without refresh.
- Refresh failure returns a reauthorization-style error.

Frontend/demo tests should cover:

- Gmail sign-in no longer calls `window.prompt`.
- Gmail sign-in enters waiting state and completes through the API.
- Demo OAuth completion rejects wrong state.
- Existing generic IMAP/SMTP account save/test flow still renders password fields and uses password auth.

Verification should run:

```bash
cargo fmt --all --check
cargo test -p app-api
cargo test -p mail-core -p mail-store -p mail-protocol
pnpm test
pnpm build
```

## Acceptance Criteria

- A user can click `SIGN IN WITH GOOGLE`, finish Google authorization in the browser, and return to AgentMail without pasting an authorization code.
- Gmail sync and Gmail send keep working after access token expiry by refreshing the token automatically.
- Wrong OAuth state fails in both backend tests and browser demo tests.
- Password-based IMAP/SMTP behavior is unchanged.
- Public Gmail release remains documented as blocked on Google consent/app verification, but internal testing can proceed.
