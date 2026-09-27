# Authelia single sign-on

Connect an Authelia user to your existing StatusDeck administrator account. Local
email/password sign-in remains available. StatusDeck supports one configured
provider at a time.

## 1. Prepare HTTPS and the OIDC provider

These examples use `https://statusdeck.example.com` for StatusDeck and
`https://auth.example.com` for Authelia. Substitute your real HTTPS addresses.
Configure StatusDeck's `STATUSDECK_BASE_URL`, `STATUSDECK_TRUSTED_HOSTS`, and
`STATUSDECK_SESSION_COOKIE_SECURE=true`; see [deployment guidance](deployment.md).
The StatusDeck backend must reach Authelia and trust its HTTPS certificate.

Authelia must have its OIDC provider enabled, including its HMAC secret and an
RSA signing key for **RS256**. If you have only configured Authelia as a reverse
proxy authentication service, complete the
[OIDC provider setup](https://www.authelia.com/configuration/identity-providers/openid-connect/provider/)
first. Keep the provider's signing keys in Authelia, not in StatusDeck.

## 2. Generate a client secret

Run this with the Authelia CLI:

```sh
authelia crypto hash generate pbkdf2 --variant sha512 --random --random.length 72 --random.charset rfc3986
```

Keep both outputs private. Put the **hash** in Authelia's client configuration;
put the original **random password** in StatusDeck's Client secret field. Do not
paste the hash into StatusDeck.

See [Authelia's client secret guidance](https://www.authelia.com/integration/openid-connect/frequently-asked-questions/).

## 3. Register StatusDeck in Authelia

Merge this client into your existing `identity_providers.oidc.clients` list.
Replace the hash placeholder and copy StatusDeck's exact Redirect URI into
`redirect_uris`. Preserve your other clients and provider settings.

```yaml
identity_providers:
  oidc:
    clients:
      - client_id: 'statusdeck'
        client_name: 'StatusDeck'
        client_secret: 'REPLACE_WITH_GENERATED_PBKDF2_HASH'
        public: false
        authorization_policy: 'two_factor'
        redirect_uris:
          - 'https://statusdeck.example.com/api/v1/auth/oidc/callback'
        scopes:
          - 'openid'
        grant_types:
          - 'authorization_code'
        response_types:
          - 'code'
        require_pkce: true
        pkce_challenge_method: 'S256'
        id_token_signed_response_alg: 'RS256'
        token_endpoint_auth_method: 'client_secret_basic'
```

The example requires users to complete two-factor authentication. Validate your
Authelia configuration and restart Authelia to apply it. See
[Authelia's client settings](https://www.authelia.com/configuration/identity-providers/openid-connect/clients/).

## 4. Configure StatusDeck

Open the discovery document:

```text
https://auth.example.com/.well-known/openid-configuration
```

Copy its exact `issuer` value. Sign in locally to StatusDeck, open
**Profile → Single sign-on**, enable OIDC, and enter:

| Field | Value |
| --- | --- |
| Provider name | Authelia |
| Issuer URL | Exact issuer from discovery, usually `https://auth.example.com` |
| Client ID | `statusdeck` |
| Client secret | Original random password from step 2, not its hash |
| Discovery URL | Leave blank for standard discovery |
| Client authentication method | HTTP header (`client_secret_basic`) |
| Additional trusted origins | Leave blank if endpoints use the issuer's origin |
| Session lifetime | `28800` seconds, or a shorter duration |
| Private CA certificate | Leave blank for public HTTPS; otherwise use the issuing CA certificate in PEM format |

Enter your local StatusDeck password and click **Save changes**. StatusDeck
requests only `openid` and uses the token's issuer and subject to identify your
linked account. No email, group, or custom claims mapping is needed.

## 5. Link your account and test

1. Enter your local StatusDeck password under **Link your account**, then click
   **Link with SSO provider**.
2. Sign in to Authelia and complete its authentication and consent prompts.
3. On return, verify the account and click **Confirm account**. This signs out
   your other StatusDeck sessions.
4. Sign out and test **Sign in with Authelia** on the StatusDeck login page.

This client example does not configure back-channel logout. Do not add
StatusDeck's Back-channel logout URL to `redirect_uris`. Sign out of StatusDeck
separately; do not assume ending an Authelia session also ends its StatusDeck
session.

## Troubleshooting

- **Invalid client:** Check that StatusDeck has the original secret and Authelia
  has the corresponding hash, with matching client IDs and authentication method.
- **Redirect mismatch:** Copy StatusDeck's full Redirect URI without wildcards.
- **Provider unavailable:** Check discovery, exact issuer, RS256 signing keys,
  backend connectivity, and HTTPS trust.
- **Cannot manage settings:** Use your local StatusDeck email and password.
- **Switching providers:** Disconnect the existing linked account and link the
  intended user after saving the new provider settings.
