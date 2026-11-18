# Security

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's private
[**Report a vulnerability**](https://github.com/yohannaftali/telepad/security/advisories/new)
form instead, with the Telepad version, your OS and steps to reproduce. You will get an
answer in the advisory thread; fixes are released as a new version, which running apps
offer through the in-app updater.

Only the latest release is supported.

## Security model

- Monitoring only runs read-only commands on hosts; `sudo` is only used as
  `sudo -n` (never prompts) for the BMC readings. Changes happen only through actions
  the user starts and confirms: Docker / BMC actions in a terminal, and **Public keys…**
  (adds or removes one line in the login user's `~/.ssh/authorized_keys` after the user
  types `add` / `revoke`; it refuses to revoke the key Telepad itself logs in with).
- New host keys are accepted on first connect (`StrictHostKeyChecking=accept-new`);
  a **changed** host key is rejected, as with plain ssh.
- Host fields are validated before they are saved, because the database is rendered
  into an ssh config; values are passed to ssh as separate arguments, never through a shell.
- Updates are downloaded only from this repository's GitHub release downloads and
  checked against the published SHA-256 before anything is replaced.
- Telepad stores no passwords or keys: it uses the OpenSSH client and the key files
  your ssh config already names. `telepad.db` holds host names, addresses, users and
  key *paths*, so treat it like an ssh config and do not share it.
