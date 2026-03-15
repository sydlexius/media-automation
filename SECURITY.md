# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| main    | Yes       |

## Reporting a Vulnerability

**Please do not open a public issue for security vulnerabilities.**

Use GitHub's [private vulnerability reporting](https://github.com/sydlexius/media-automation/security/advisories/new) to report security issues. You'll receive a response within 72 hours.

When reporting, please include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

## Scope

This project interacts with Emby and Jellyfin media server APIs using API keys. Security concerns include:

- API key exposure (keys should only be in `.env` files or environment variables, never committed to source control)
- Server-side request forgery (SSRF) via user-supplied server URLs

## Security Measures

- API keys are loaded from `.env` files (gitignored) or environment variables
- `.env.example` contains no real credentials
- Dependabot alerts and automated security fixes are enabled
- CodeQL analysis runs on every push to main
