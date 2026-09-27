import { Link } from "react-router-dom";
import { MarkdownContent } from "../../components/MarkdownContent";
import authelia from "../../../../docs/authelia.md?raw";
import authentik from "../../../../docs/authentik.md?raw";
import keycloak from "../../../../docs/keycloak.md?raw";

const providers = [
  { id: "authelia", name: "Authelia", instructions: authelia },
  { id: "authentik", name: "Authentik", instructions: authentik },
  { id: "keycloak", name: "Keycloak", instructions: keycloak },
];

export default function SsoInstructions() {
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>SSO setup instructions</h1>
        </div>
        <Link className="button ghost" to="/profile?section=sso">
          Back to single sign-on
        </Link>
      </div>
      <nav className="card" aria-labelledby="sso-contents">
        <h2 id="sso-contents">Table of contents</h2>
        <ul>
          {providers.map(({ id, name }) => (
            <li key={id}>
              <a href={`#${id}-setup`}>{name}</a>
            </li>
          ))}
        </ul>
      </nav>
      {providers.map(({ id, name, instructions }) => (
        <article
          key={id}
          className="card sso-provider-guide"
          id={`${id}-setup`}
          aria-labelledby={`${id}-title`}
          tabIndex={-1}
        >
          <h2 id={`${id}-title`}>{name}</h2>
          <MarkdownContent
            value={instructions
              .replace(/^# .+\n/, "")
              .replace(/^## /gm, "### ")
              .replace(
                "[deployment guidance](deployment.md)",
                "your deployment documentation",
              )}
          />
        </article>
      ))}
    </>
  );
}
