# Keycloak single sign-on

Connect a Keycloak user to your existing StatusDeck administrator account. Local
email/password sign-in remains available. StatusDeck supports one configured
provider at a time.

## 1. Create the Keycloak client

In your application realm, open **Clients → Create client**:

| Setting | Value |
| --- | --- |
| Client type | OpenID Connect |
| Client ID | `statusdeck` |
| Client authentication | On |
| Standard flow | On |
| Implicit flow, Direct access grants, Service accounts roles | Off |
| Valid redirect URIs | Copy the exact Redirect URI from StatusDeck |
| Home URL | `https://statusdeck.example.com` |

Save. Under **Credentials**, use **Client Id and Secret** and copy the client
secret. In the client's advanced settings, select **RS256** for ID token signing
and **S256** for the PKCE method. Leave ID token encryption disabled.

Use an exact callback address, such as
`https://statusdeck.example.com/api/v1/auth/oidc/callback`, without wildcards.

See [Keycloak's client configuration guide](https://www.keycloak.org/docs/latest/server_admin/#_oidc_clients).

## 2. Configure StatusDeck

Open the realm's discovery document:

```text
https://keycloak.example.com/realms/statusdeck/.well-known/openid-configuration
```

Copy its exact `issuer` value. Include any deployment-specific path prefix;
this example's issuer is `https://keycloak.example.com/realms/statusdeck`.
See [Keycloak's OIDC endpoints](https://www.keycloak.org/securing-apps/oidc-layers).

Sign in locally to StatusDeck and open **Profile → Single sign-on**. Enable OIDC
and enter:

| Field | Value |
| --- | --- |
| Provider name | Keycloak |
| Issuer URL | Exact issuer from discovery |
| Client ID | `statusdeck` |
| Client secret | Secret copied from the Credentials tab |
| Discovery URL | Leave blank for the standard realm discovery URL |
| Client authentication method | HTTP header (`client_secret_basic`) |
| Additional trusted origins | Leave blank if all endpoints use the issuer's origin |
| Session lifetime | `28800` seconds, or a shorter duration |
| Private CA certificate | Leave blank for public HTTPS; otherwise use the issuing CA certificate in PEM format |

Confirm the displayed Redirect URI matches the client registration. Enter your
local StatusDeck password and click **Save changes**. StatusDeck requests
`openid`, validates RS256 ID tokens, and identifies your account by issuer and
subject. No email mapping or automatic account creation is needed.

## 3. Link your account and test

1. Enter your local StatusDeck password under **Link your account**, then click
   **Link with SSO provider**.
2. Sign in as the intended Keycloak user.
3. Return to StatusDeck, verify the account, and click **Confirm account**.
   This signs out your other StatusDeck sessions.
4. Sign out, then use **Sign in with Keycloak** on the StatusDeck login page.

## 4. Optional: enable logout notifications

In the Keycloak client's logout settings, turn **Front channel logout** off.
Set **Backchannel logout URL** to StatusDeck's displayed **Back-channel logout
URL** and enable **Backchannel logout session required**. Save the client.

Keycloak must reach StatusDeck's endpoint. End the Keycloak session and verify
that the matching StatusDeck SSO session ends. Signing out of StatusDeck alone
ends only its own session.

See [Keycloak's logout settings](https://www.keycloak.org/docs/latest/server_admin/#_oidc_clients).

## Troubleshooting

- **Redirect mismatch:** Copy the full Redirect URI, including its path and port.
- **Provider unavailable:** Check the realm, issuer, discovery URL, RS256 signing,
  backend connectivity, and certificate trust.
- **Client authentication fails:** Recheck the client ID, secret, and HTTP header
  authentication method.
- **Cannot manage settings:** Sign in using your local StatusDeck password.
- **Switching providers:** Disconnect the existing linked account and link the
  intended user after saving the new provider settings.
