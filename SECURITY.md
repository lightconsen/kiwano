# Security Policy

Kiwano's whole premise is that your provider keys stay on your machine. If you
find a way for a key, a request body or a log entry to end up somewhere it
shouldn't, that is the bug we care about most — please tell us privately.

## Supported versions

Only the latest release line (currently 0.1.x) receives fixes. Please update
before reporting, so we are not chasing something already fixed.

## Reporting a vulnerability

**Please do not open a public issue, and do not post it in Discussions.**

Use GitHub's private vulnerability reporting instead:

https://github.com/lightconsen/kiwano/security/advisories/new

What makes a report actionable:

- the version and the platform;
- the steps to reproduce;
- what it actually buys an attacker — reading a stored key? capturing a request
  body? code execution? Local-only issues still matter here, because the
  threat model is "someone else's code on your machine".

We aim to acknowledge within 72 hours. If the report is accepted, we will
coordinate a fix and a release with you, and credit you in the advisory unless
you would rather stay anonymous.

## Scope

In scope:

- credential storage — the local SQLite store, its file and directory
  permissions, the platform hardening applied on Windows;
- the local gateway on `127.0.0.1:8317`, including `/metrics` and its token
  handling, and any path that lets a non-local caller reach it;
- request and response bodies held in the request log;
- the update channel and its minisign verification;
- the Hub sync path.

Out of scope:

- a vulnerability in a provider you configured yourself;
- anything that already requires root, administrator or physical access;
- missing hardening on data the user themselves chose to export or share.

## Already in place

- Keys live in an owner-only local database: directory `0700`, file `0600`,
  with additional hardening on Windows.
- The gateway binds to loopback. `/metrics` hashes agent labels by default and
  can require a bearer token for full names.
- Every release ships `SHA256SUMS` and verifiable build provenance:

  ```bash
  gh attestation verify <file> --repo lightconsen/kiwano
  ```

- No telemetry. The update check is a single HTTPS request for a static
  manifest; nothing else leaves the machine except the requests you make to
  your own providers.
