# Telepad user manual

How to use Telepad once it is [installed](../README.md#install). For a first run see the
[quick start](../README.md#quick-start).

**Contents:** [Window layout](#window-layout) · [Overview](#overview) · [Host detail](#host-detail) · [Refreshing](#refreshing) · [Adding, editing and removing hosts](#adding-editing-and-removing-hosts) · [Import and export](#import-and-export) · [Fan speeds and BMC temperatures (IPMI)](#fan-speeds-and-bmc-temperatures-ipmi) · [Updates](#updates) · [Troubleshooting](#troubleshooting) · [Command line](#command-line)

## Window layout

| Area | What it shows |
|---|---|
| **Top bar** | **Refresh all**, **Auto every _N_ s**, theme, the search box, up/down/total counts, a spinner while hosts load, an **Update X available** button when a new release exists, and at the top right the **☰** menu (add, import, export, software update) |
| **Host list** (left) | The large **Overview — all hosts** and **Overview — Docker** buttons (the one for the page on screen is filled), the **View** switch, then every host (see [Host list](#host-list)) |
| **Main area** | The overview table, or the detail page of the selected host |
| **Status bar** (bottom) | Path of the database in use (hover: the generated ssh config), the Telepad version, and short messages |

Status dot: green = last check succeeded, red = it failed, blue = first check running,
grey = no result yet (or not monitored). Usage bars turn yellow and red as they fill; temperatures are blue
below 70 °C, yellow from 70 °C and red from 85 °C.

## Theme

**Theme** in the top bar switches between **System** (follows the Windows / macOS / desktop
setting, the default), **Light** and **Dark**. The choice is saved in the database.

## Host list

The list on the left has two views, switched with **View: Compact / Detailed** at its top:

- **Compact** (default): one line per host — status dot, OS logo, alias, and on the right the
  highest of CPU, RAM and fullest-disk usage (e.g. `Disk 87%`, coloured like the bars).
  Hover over the bar for all three values, over a red warning sign for the error, and over
  the name for the description and address.
- **Detailed**: three lines per host — alias and address, description, and CPU / RAM / disk
  bars (or the error).

Tick **Group by status** to split the list into **Down**, **Up** and **Not checked yet**
(down hosts first, each group collapsible, with its count). Click a host to open it;
right-click for Edit / Clone / Delete. Both choices are saved in `telepad.db` and come back
the next time Telepad starts.

**Reorder hosts:** drag a host by its **⋮⋮** handle (left of the status dot) and drop it on
the upper or lower half of another host; a line shows where it will land. The order is
saved in `telepad.db` at once, used by the list, the table, cards and the generated ssh
config, and kept after a restart. The handles are hidden while the search box has text or
**Group by status** is ticked (a hint says so), because the position would be ambiguous.

## Overview

Click **Overview — all hosts** at the top of the host list to see every host at once. The
**View** switch above the overview chooses **Table** or **Cards**; the choice is remembered.

### Docker overview

**Overview — Docker** (below **Overview — all hosts**) lists the containers of every host
grouped by stack (the compose project name), stacks A–Z, then **Standalone containers**. A
stack running on several hosts appears once; inside it, each host has its own line with a
link to the host, how the stack is managed (docker compose folder or Portainer stack) and
the same **Compose** menu / **Open in Portainer** button as on the host page. Each container
shows image, status, container IPs and published ports, and its **☰** menu has the usual
[Docker actions](#docker-actions). Click a stack name to fold it. The search box filters by
stack, container, image or host.

### Table

One row per host: **Host**, **Description**, **Address**, **OS**, **Flags**, **CPU**, **Memory**,
**Fullest disk**, **Temp** (hottest sensor), **Docker** (running/total containers) and
**Updated** (age of the data). Click a host name to open its detail page; the button at
the end of a row opens SSH/RDP.

In inventory order (no column sorted, no search) each row has a **⋮⋮** handle in the first
column: drag it to reorder hosts, as in the host list. While a column is sorted or the
search box has text, the handles are hidden and a hint above the table says what to clear.

- **Sort:** click a column header. Numbers start with the highest (busiest CPU, fullest
  disk, hottest, most containers, most recently updated), text columns from A to Z; the
  arrow next to the header shows the direction. Click again to reverse, a third time to
  return to the inventory order. The unlabelled first column sorts by status (down hosts
  first). Addresses sort numerically (`10.0.0.9` before `10.0.0.10`), and hosts without a
  value for the column (no data yet, or unreachable) always stay at the bottom.
- **Resize:** drag the border between two column headers. Text that does not fit is cut
  with "…"; hover over it to read all of it. Error messages stay inside the OS column.
- The header row stays in place while you scroll; when the columns are wider than the
  window, scroll sideways.

The search box filters both the host list and the overview by host name, address, OS
or container name.

### Cards

One card per host, wrapping to the window width: status, OS logo, alias (click it for the
detail page), address and description, OS, CPU / RAM / fullest-disk bars, hottest
temperature, Docker summary, how long ago it was checked, and an **SSH** / **RDP** button.
Unreachable hosts show their error instead of the bars. The search box filters the cards;
**Sort by** picks the order (the same columns as the table, or inventory order) and the
arrow button next to it reverses it.

### Maintenance flags

The **Flags** column, the cards, the compact host list and the host's **Maintenance**
section show what needs doing on a host (hover for details):

| Flag | Meaning | Source on the host (read-only, offline) |
|---|---|---|
| red **EOL** | the OS release is past its vendor's end of support: no more security fixes | `/etc/os-release` `ID` + `VERSION_ID` against Telepad's built-in table |
| yellow **EOL 75d** | support ends within 6 months | same |
| yellow **reboot** | a reboot is pending (with the packages that asked for it) | `/var/run/reboot-required`, `needs-restarting -r`, or the running kernel older than the newest installed one |
| **⬇ 12 (3 sec)** | 12 package updates waiting, 3 of them security updates (red when there are security updates) | `apt-check` / `apt-get -s`, `dnf` / `yum check-update --cacheonly`, `apk` |

Update counts come from the host's local package cache, so they are only as fresh as the
host's last `apt update` / `dnf makecache`. Releases not in the table show "unknown"
support. Sort by **Flags** to see the most urgent hosts first.

## Host detail

The header shows the status, OS logo, alias and `user@address`. Below it are the buttons
**Open SSH** / **Open RDP**, **Copy ssh command**, **Refresh**, then **Edit**, **Clone** and
**Delete**. On a narrow window the buttons (and the top bar) wrap onto a second line instead
of being cut off. Below that:

- **System**: OS, kernel, architecture, uptime, CPU model and cores, load average.
- **Usage**: CPU, Memory and Swap bars.
- **Disks**: every mount with size, used and free space.
- **Network**: interfaces and their IP addresses.
- **Sensors**: temperatures and fan speeds. Fans spin faster in the drawing the faster
  they run. On a physical server whose BMC is not readable yet, **Set up BMC access…**
  appears (see [Fan speeds and BMC temperatures](#fan-speeds-and-bmc-temperatures-ipmi)).
- Under the description: when the OS / CPU facts and the addresses / ports / updates were
  last read (see [Refreshing](#refreshing)).
- **Docker**: runtime version, management (for example a Portainer agent), compose
  stacks with their folder and file, and every container with image, state, container
  IP per network and published ports. See [Docker actions](#docker-actions).
- **Ports in use**: every listening port with the container or process that owns it
  (the process only when the login user may see it); **nginx only** narrows the list to
  the ports nginx serves.

## Docker actions

Each container row has a **☰** menu, and each docker compose stack a **Compose** menu.
Telepad never changes a container by itself: every action opens a terminal on the host
that prints the exact command and runs it only after you press **Enter**. **Kill**,
**Remove** and compose **Down** ask you to type `yes`. If your user may not use Docker
directly, the command runs with `sudo` and sudo asks for your password there.

| Menu | Actions |
|---|---|
| Container ☰ | Shell (`docker exec -it`, bash if present), Logs (`logs -f --tail 200`), Start, Stop, Restart, Pause, Resume, Kill, Remove (`rm` without `-f`: stop it first; volumes are kept) — only the ones that fit the container's state |
| Compose | Logs, Up (`up -d`), Restart, Stop, Down (`down` without `-v`: named volumes are kept), run with the stack's own folder, project name and files |
| Portainer stack | **Open in Portainer** when the Portainer server runs on this host, otherwise "change it in Portainer": Portainer stacks are only changed in Portainer |

**How it runs…** (container ☰) reads `docker inspect` and the stack's compose files
(read-only) and shows a `docker run` command rebuilt from them, with **Copy** buttons.
Image defaults (environment, command) appear as options, and values whose name looks
like a secret (`PASSWORD`, `TOKEN`, `KEY`, …) or passwords inside URLs are shown as `***`.
Portainer keeps its stacks' compose files inside its own volume, so they are not shown.

Example (made-up container):

```
docker run \
  -d \
  --name shop-db \
  --restart unless-stopped \
  --network shop_default \
  -p 127.0.0.1:3307:3306 \
  -v shop_db:/var/lib/mysql \
  -e 'MARIADB_ROOT_PASSWORD=***' \
  mariadb:11
```

## Swarm and Kubernetes

When a host is a **Docker Swarm** manager, or can read a **Kubernetes** cluster with
`kubectl`, `k3s kubectl` or `microk8s kubectl` (its own kubeconfig, or `sudo -n` when
allowed), the host page gets a **Cluster** section and the overview's Docker cell a badge
(*Swarm manager*, *Swarm worker*, *k8s*).

- **Swarm**: nodes (status, availability, leader), services with replicas running /
  wanted, stacks, and tasks that should run but do not. Workers only show their role: the
  cluster can only be read on a manager.
- **Kubernetes**: nodes (ready), deployments, statefulsets and daemonsets with ready /
  wanted pods, and problem pods (not running, waiting such as `CrashLoopBackOff`, or
  restarted 5 times or more).
- A degraded cluster (node down, missing replicas, problem pods) raises a red
  **cluster** flag in the Flags column.
- Several managers of one cluster, or one machine under two aliases, show the cluster
  once; the others say "Same cluster as …" with a link.

Actions work like the [Docker actions](#docker-actions): a terminal shows the exact
command and runs it after you confirm.

| Where | Actions |
|---|---|
| Swarm service ☰ | Logs (`docker service logs -f`), Restart (rolling, `service update --force`), Scale… (the terminal asks for the number of replicas) |
| Swarm node ☰ | Drain, Pause, Activate (`docker node update --availability`) |
| Swarm stack ☰ | Remove stack (`docker stack rm`, type `yes`) |
| Kubernetes workload ☰ | Logs, Rollout restart, Scale… (not for daemonsets) |

Collection stays read-only, every cluster command has a time limit, and long lists are
capped (500 services, 1000 workloads per kind, 100 problem pods).

## Refreshing

All monitored hosts are checked at start, then every 60 s while **Auto every** is ticked
(change the interval next to it, 15–3600 s). **Refresh all** checks every host now;
**Refresh** on a host checks only that one.

Not everything is read on every round. Automatic rounds read:

| What | How often |
|---|---|
| Uptime, load, CPU, memory, disks, sensors, Docker, Swarm, Kubernetes | every round (**Auto every**) |
| IP addresses, listening ports, pending updates, reboot needed | every 6 hours |
| Hostname, OS release, kernel, architecture, CPU model | once a day |

**Refresh** and **Refresh all**, a new or edited host and a host that has just rebooted
(uptime went down) read everything. A host that is down and does not answer ping is only
pinged on the following rounds (no ssh connection, no timeout to wait for); as soon as it
answers ping everything is read again, and ssh is still tried every 10 minutes for hosts
whose firewall drops ping. Remote Desktop hosts are always just ping + port check.

Up to 12 hosts are checked at the same time,
each with an 8 s connect timeout and a 40 s total timeout, so one slow host never holds
up the others. The window stays responsive while hosts load.

## Adding, editing and removing hosts

**☰ → Add one host…** opens the editor:

| Field | Meaning |
|---|---|
| **Alias (Host)** | Name used in ssh and in Telepad: letters, digits, `.`, `_`, `-` |
| **Description** | Free text shown under the name |
| **HostName / IP** | Address to connect to (empty = the alias itself) |
| **User**, **Port**, **IdentityFile** | As in an OpenSSH config; empty = ssh's defaults. For IdentityFile, the arrow next to the field lists the private keys found in `~/.ssh`; **Browse…** opens in `~/.ssh` (on macOS press Cmd+Shift+. in the dialog to see other hidden folders); typing `~/.ssh/id_ed25519` works too |
| **Access** | **SSH (monitored)** or **RDP only (Windows)** |
| **Monitored** | Untick to keep a host in the list (for SSH/RDP buttons) without polling it |
| **Other ssh options** | One `Keyword value` per line, e.g. `ProxyJump bastion` or `KexAlgorithms +diffie-hellman-group1-sha1` |

**Edit** changes a host, **Clone** starts a new host with the same User, Port and
IdentityFile, and **Delete** removes it after a confirmation. Right-click a host in the
list for the same actions. Every change regenerates `telepad.ssh_config` immediately.

**Windows servers without SSH:** with Access set to **RDP only**, Telepad never opens an
SSH connection to the host. It pings it and tests the RDP port (3389, or the Port you
set); the host is up if either answers. **RDP** opens Remote Desktop (`mstsc` on Windows).

When an SSH check fails, the error says whether the host still answers ping, so a
machine that is down looks different from one where only SSH is broken.

## Import and export

- **☰ → Import ~/.ssh/config** imports your own OpenSSH client config in one click.
- **☰ → Import from a file…** reads any file you pick:
  - an **OpenSSH client config** (any file that is not JSON);
  - **JSON** with a list of hosts: a Telepad export from any version, `{"hosts": [...]}`
    or a bare `[...]`. Common field names are understood, in any case and with or
    without `_`/`-`: `alias`/`name`/`host`, `hostname`/`ip`/`address`, `user`/`username`,
    `port`, `identity_file`/`key`, `description`/`note`/`comment`, `access` (`ssh`/`rdp`),
    `monitored`/`unmonitored`, and `options` as `[["Keyword","value"]]` or `{"Keyword":"value"}`.
    A host needs at least a name or an address.

  Either way a preview comes first:
  - **Merge — add new, update same alias, keep others**: new hosts are added at the
    end of the list in imported order; updated hosts keep their place.
  - **Replace — hosts become exactly this list**, in the order of the import.

  An import is all or nothing: if any host is invalid, nothing changes. Global
  `Host *` / `Match` blocks are imported too.
- **☰ → Import from pasted text…** opens a text box: paste with Ctrl+V or
  **Paste from clipboard**, then **Next: preview**. It reads OpenSSH config text
  (`Host` blocks) or JSON as above, or a simple list with one host per line:

  ```
  web01 admin@192.0.2.10
  nas01 192.0.2.20:2222
  198.51.100.7
  ```

  that is `[alias] [user@]host[:port]` (without an alias the address becomes the
  alias; port 22 when none is given). Blank lines and `#` comments are skipped.
  Lines that cannot be read and invalid hosts are left out and listed with their
  line number in the preview; the other hosts can still be imported. If nothing
  can be read, the window stays open and says why.
- **☰ → Export…** writes `.json` (re-importable, keeps every field) or OpenSSH
  config text. Hosts that are not monitored or RDP-only are marked with
  `# telepad: not monitored` / `# telepad: rdp only` comments so they round-trip.

`telepad.db` is a normal SQLite file: back it up by copying it while Telepad is closed,
or use Export. You can also inspect it with any SQLite tool.

## Public keys (add / revoke)

**☰ → Public keys… (add / revoke)** gives someone SSH access to many hosts at once, or
takes it away, using the access Telepad already has. It changes only the
`~/.ssh/authorized_keys` file of the user Telepad logs in as on each host (no `sudo`).

1. Paste one public key: the single line from a `.pub` file, for example
   `ssh-ed25519 AAAAC3Nza… alice@example.com`. Telepad shows its type, its SHA256
   fingerprint (the same as `ssh-keygen -lf`) and its comment, or why it is not a valid
   key. Private keys, several lines and keys with options (`from=`, `command=`) are refused.
2. Tick the hosts. **Select all** ticks every host the filter shows (click again to clear
   them); the filter box narrows the list by name, address, description or OS. RDP-only
   hosts are listed but cannot be ticked. The number of selected hosts is shown.
3. Click **Add key** or **Revoke key**. Telepad lists the key and every selected host;
   type `add` or `revoke` and press Enter (or the button) to start, or **Cancel**.
4. The hosts are done in parallel; each shows its result:
   - **added** / **already present (not changed)** for Add,
   - **removed** / **not present (not changed)** for Revoke,
   - **refused** — an RDP-only host, or the key Telepad itself logs in with (revoking it
     would lock Telepad out of that host),
   - **failed** with the reason (host down, `~/.ssh` not writable, …).

   The status bar sums it up when all hosts are done.

Add creates `~/.ssh` (mode 700) and `authorized_keys` (mode 600) when they are missing
and never adds a key twice. Revoke removes every line with that key, whatever its comment
or options. Other lines are never touched, and the file is rewritten through a temporary
copy, so a failure leaves it as it was.

## Fan speeds and BMC temperatures (IPMI)

CPU temperatures work without setup. Fans and inlet/exhaust temperatures on physical
servers come from the BMC through `ipmitool`, which needs root, so each physical host
needs a one-time setup. Open the host and in **Sensors** click **Set up BMC access…**:
a terminal opens on the host showing the exact commands, waits for Enter and lets
sudo ask for your password. By hand (replace `USER` with the login user):

```bash
sudo apt install ipmitool
echo 'USER ALL=(root) NOPASSWD: /usr/bin/ipmitool sdr type Fan, /usr/bin/ipmitool sdr type Temperature' | sudo tee /etc/sudoers.d/telepad-ipmi
sudo chmod 440 /etc/sudoers.d/telepad-ipmi && sudo visudo -c
```

The rule allows only those two read-only commands. This is the only change Telepad ever
offers to make on a host, and only in a terminal you opened yourself. Virtual machines
have no BMC; the Sensors section says which step is missing on a host that has one.

## Updates

Telepad checks the latest [GitHub release](https://github.com/yohannaftali/telepad/releases)
at start and every 6 hours, in the background, over HTTPS (no account or token). It reads
github.com's normal release pages, not the GitHub API, so an office where many people
share one internet address does not run into GitHub's API limit. If a check fails, it is
tried again after 30 minutes and the reason shows in **Software update…**.

1. When a newer version exists, the top bar shows **Update X available**.
2. Click it (or **☰ → Software update…**) to see the version and a link to the
   release notes.
3. **Update now** downloads the zip for your platform, verifies it against the release's
   `.sha256` and replaces `telepad.exe` and `telepad-check.exe`. If anything fails,
   nothing is replaced.
4. **Restart now** (or **Restart for X** in the top bar) starts the new version.

**Later** hides the button until the next start; **Check now** checks immediately.
Your `telepad.db`, `config` and `.env` are never touched. Under **Update source** you
can turn the checks off or point them at a fork's repository.

**Install automatically** (under **Update source**, off by default) skips steps 1–3:
when a check finds a newer version, Telepad downloads it, verifies the `.sha256` and
installs it in the background. Nothing restarts by itself: the top bar shows
**Restart for X**, or the new version starts the next time you open Telepad. **Later**
also pauses automatic installs until the next start.

## Troubleshooting

| Message | What to check |
|---|---|
| `authentication failed` | The key is not accepted or the host wants a password. Try `ssh <alias>` in a terminal; check IdentityFile and the server's `authorized_keys`. A key with a passphrase must be loaded in the ssh agent first (macOS: `ssh-add --apple-use-keychain ~/.ssh/id_ed25519`). |
| `private key … is readable by other users` | ssh ignores private keys that others can read (common after copying keys from another computer). Run the `chmod 600 <key>` shown in the message, and `chmod 700 ~/.ssh`. |
| `host key mismatch` | The host's key changed. Verify why, then remove the old line from `~/.ssh/known_hosts`. |
| `connection timed out` + `(no ping reply)` | The host is down or unreachable from your network (VPN?). |
| `connection timed out` + `(host answers ping …)` | The machine is up but SSH is blocked or not running. |
| `no common SSH algorithm (legacy host)` | Add e.g. `KexAlgorithms +diffie-hellman-group1-sha1` under **Other ssh options**. |
| Sensors: `ipmitool not installed` / `sudo rule missing` | Use **Set up BMC access…** on that host. |
| Update check: `cannot reach GitHub` | No internet or a proxy blocks `api.github.com`; Telepad keeps working without updates. |

## Command line

```powershell
telepad.exe                              # GUI
telepad.exe --db D:\other\telepad.db     # use another database
telepad.exe --config D:\ssh\config       # import from this config on first start
telepad-check.exe                        # check all monitored hosts once, text
telepad-check.exe web01 nas01 --json     # selected hosts, JSON
telepad-check.exe --check-update         # newer release on GitHub?
```

`telepad-check` exits 0 when every host answered and non-zero otherwise, so it fits in
scripts and scheduled tasks. `--check-update` exits 0 when up to date, 10 when an update
is available and 2 when the check failed.

| Setting | Where |
|---|---|
| Database | `--db <path>`, `TELEPAD_DB`, else `telepad.db` next to `config`, else next to the exe for a portable copy (`portable` file or an existing `telepad.db` there), else the per-user data folder: `%APPDATA%\Telepad`, `~/Library/Application Support/Telepad`, `~/.local/share/telepad` |
| First-run import | `--config <path>`, `TELEPAD_CONFIG`, `./config`, `config` next to the exe, or `config` in the data folder; only read while the database is empty |
| Generated ssh config | `telepad.ssh_config` next to the database (do not edit: rewritten on every change) |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE=` in a `.env` file next to `config` |
| ssh binary | `TELEPAD_SSH` (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh` on `PATH`) |

Security model: see [SECURITY.md](../SECURITY.md).
