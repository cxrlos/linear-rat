# Security

lrat holds a Linear API key that can read and change your workspace, so I treat anything that could leak or misuse it as a security issue.

## Reporting

Report vulnerabilities privately through [GitHub security advisories](https://github.com/cxrlos/linear-rat/security/advisories/new), not as a public issue. I aim to reply within a week.

## Supported versions

Only the latest released version gets fixes.

## How lrat handles your key

- It reads the key from the `LINEAR_API_KEY` environment variable, or from your OS keyring (stored there by `lrat auth login`).
- It never writes the key to the config file or to logs.
- It sends the key only to `api.linear.app`.
