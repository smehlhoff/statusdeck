# Authentik single sign-on

Connect an Authentik account to your existing StatusDeck administrator account.
Local email/password sign-in remains available as backup.

The examples use `https://statusdeck.example.com` for StatusDeck and
`https://auth.example.com` for Authentik. Replace both with your actual HTTPS URLs.

## 1. Create the Authentik application

In Authentik's admin interface, go to **Applications → Applications → New
Application**. Create an application named **StatusDeck**, with slug `statusdeck`,
and choose **OAuth2/OIDC** as its provider type. Older versions may require
creating the provider and application separately.

Configure the provider:

| Setting | Value |
| --- | --- |
| Client type | Confidential |
| Client ID and client secret | Keep the generated values; copy them into StatusDeck in step 2 |
| Authorization flow | Your normal application authorization/consent flow |
| Redirect URI matching | Strict |
| Redirect URI | `https://statusdeck.example.com/api/v1/auth/oidc/callback` |
| Signing key | An RSA certificate/key supporting RS256 |
| Encryption key | Leave unset |
| Issuer mode | Each provider has a different issuer, based on the application slug |

Keep the default scope mappings. StatusDeck requests `openid` and uses the token's
subject to identify your account; email matching and automatic user creation are
not used. Do not leave the signing key empty: StatusDeck requires RS256.

Set the application's **Launch URL** to `https://statusdeck.example.com`.
The callback belongs in the provider's **Redirect URIs**, not the Launch URL.

See Authentik's [application creation guide](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/create-oauth2-provider)
and [provider settings](https://docs.goauthentik.io/add-secure-apps/providers/oauth2).

## 2. Configure StatusDeck

Open the discovery document in your browser:

```text
https://auth.example.com/application/o/statusdeck/.well-known/openid-configuration
```

Copy its exact `issuer` value, including the trailing slash. With the issuer mode
above, it should be `https://auth.example.com/application/o/statusdeck/`.

Sign in to StatusDeck with your local administrator account. Open
**Profile → Single sign-on**, enable **Single sign-on (OIDC)**, and enter:

| Field | Value |
| --- | --- |
| Provider name | Authentik |
| Issuer URL | The exact issuer copied from the discovery document |
| Client ID / client secret | Values from your Authentik provider |
| Discovery URL | Leave blank with per-provider issuer mode |
| Client authentication method | HTTP header (`client_secret_basic`) |
| Additional trusted origins | Leave blank when all provider endpoints use the issuer's origin |
| Session lifetime | `28800` seconds (eight hours), or a shorter duration |
| Private CA certificate | Leave blank for publicly trusted HTTPS; otherwise paste your issuing CA certificate in PEM format |

The read-only **Redirect URI** is generated from StatusDeck's base URL. Confirm
it matches the URI registered in Authentik. Enter your local StatusDeck password
and click **Save changes**.

## 3. Link your account and test

1. Under **Link your account**, enter your local StatusDeck password and click
   **Link with SSO provider**.
2. Sign in to Authentik as the account you want to link.
3. After returning to StatusDeck, verify the account and click **Confirm account**.
   Confirmation signs out your other StatusDeck sessions.
4. Sign out of StatusDeck, then click **Sign in with Authentik** on its login page.

The Authentik homepage tile opens StatusDeck. If signed out, use the Authentik
button on the StatusDeck login page to start SSO.

## 4. Optional: enable logout notifications

If your Authentik version supports back-channel logout, edit the provider and set:

- **Logout Method:** Back-channel.
- **Logout URI:** Copy StatusDeck's read-only **Back-channel logout URL**, normally
  `https://statusdeck.example.com/api/v1/auth/oidc/backchannel-logout`.

Authentik must be able to reach this URL. Test by ending the Authentik session and
checking that the corresponding StatusDeck SSO session ends. Signing out of
StatusDeck alone ends only its own session.

See [Authentik's logout guide](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/frontchannel_and_backchannel_logout/).

## Troubleshooting

- **Redirect mismatch:** Copy the full Redirect URI from StatusDeck into Authentik
  using strict matching. Include the callback path and any non-default port.
- **Provider unavailable:** Check the discovery URL, exact issuer, RSA signing
  key, backend DNS/network access, and HTTPS certificate trust. Discovery retries
  are limited to once per minute.
- **Global issuer mode:** Prefer per-provider mode. If global mode is required,
  use the discovery document's issuer and enter its application-specific discovery
  URL explicitly in StatusDeck.
- **Cannot manage settings:** Sign in with your local email and password first.
- **Need to disable SSO:** Turn off the toggle and save. This ends SSO sessions
  while retaining the linked account. Disconnecting removes the account link.
