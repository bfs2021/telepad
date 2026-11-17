# Telepad user manual

How to use Telepad once it is [installed](../README.md#install). For a first run see the
[quick start](../README.md#quick-start).

**Contents:** [Window layout](#window-layout) · [Overview](#overview) · [Host detail](#host-detail) · [Refreshing](#refreshing) · [Adding, editing and removing hosts](#adding-editing-and-removing-hosts) · [Import and export](#import-and-export) · [Fan speeds and BMC temperatures (IPMI)](#fan-speeds-and-bmc-temperatures-ipmi) · [Updates](#updates) · [Troubleshooting](#troubleshooting) · [Command line](#command-line)

## Window layout

| Area | What it shows |
|---|---|
| **Top bar** | **☰ Hosts** menu, **Refresh all**, **Auto every _N_ s**, the search box, up/down/total counts, a spinner while hosts load, and an **Update X available** button when a new release exists |
| **Host list** (left) | The large **Overview — all hosts** button, the **View** switch, then every host (see [Host list](#host-list)) |
| **Main area** | The overview table, or the detail page of the selected host |
| **Status bar** (bottom) | Path of the database in use (hover: the generated ssh config), the Telepad version, and short messages |

Status dot: green = last check succeeded, red = it failed, blue = first check running,
grey = no result yet (or not monitored). Usage bars turn yellow and red as they fill; temperatures are blue
below 70 °C, yellow from 70 °C and red from 85 °C.

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

## Overview

Click **Overview — all hosts** at the top of the host list to see every host at once. The
**View** switch above the overview chooses **Table** or **Cards**; the choice is remembered.

### Table

One row per host: **Host**, **Description**, **Address**, **OS**, **CPU**, **Memory**,
**Fullest disk**, **Temp** (hottest sensor), **Docker** (running/total containers) and
**Updated** (age of the data). Click a host name to open its detail page; the button at
the end of a row opens SSH/RDP.

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
- **Docker**: runtime version, management (for example a Portainer agent), compose
  stacks with their folder and file, and every container with image, state, container
  IP per network and published ports.
- **Ports in use**: every listening port with the container or process that owns it
  (the process only when the login user may see it); **nginx only** narrows the list to
  the ports nginx serves.

## Refreshing

All monitored hosts are checked at start, then every 60 s while **Auto every** is ticked
(change the interval next to it, 15–3600 s). **Refresh all** checks every host now;
**Refresh** on a host checks only that one. Up to 12 hosts are checked at the same time,
each with an 8 s connect timeout and a 40 s total timeout, so one slow host never holds
up the others. The window stays responsive while hosts load.

## Adding, editing and removing hosts

**☰ Hosts → Add one host…** opens the editor:

| Field | Meaning |
|---|---|
| **Alias (Host)** | Name used in ssh and in Telepad: letters, digits, `.`, `_`, `-` |
| **Description** | Free text shown under the name |
| **HostName / IP** | Address to connect to (empty = the alias itself) |
| **User**, **Port**, **IdentityFile** | As in an OpenSSH config; empty = ssh's defaults |
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

- **☰ Hosts → Import ~/.ssh/config** imports your own OpenSSH client config in one click.
- **☰ Hosts → Import from a file…** reads any file you pick:
  - an **OpenSSH client config** (any file that is not JSON);
  - **JSON** with a list of hosts: a Telepad export from any version, `{"hosts": [...]}`
    or a bare `[...]`. Common field names are understood, in any case and with or
    without `_`/`-`: `alias`/`name`/`host`, `hostname`/`ip`/`address`, `user`/`username`,
    `port`, `identity_file`/`key`, `description`/`note`/`comment`, `access` (`ssh`/`rdp`),
    `monitored`/`unmonitored`, and `options` as `[["Keyword","value"]]` or `{"Keyword":"value"}`.
    A host needs at least a name or an address.

  Either way a preview comes first:
  - **Merge — add new, update same alias, keep others**
  - **Replace — hosts become exactly this file**

  An import is all or nothing: if any host is invalid, nothing changes. Global
  `Host *` / `Match` blocks are imported too.
- **☰ Hosts → Export…** writes `.json` (re-importable, keeps every field) or OpenSSH
  config text. Hosts that are not monitored or RDP-only are marked with
  `# telepad: not monitored` / `# telepad: rdp only` comments so they round-trip.

`telepad.db` is a normal SQLite file: back it up by copying it while Telepad is closed,
or use Export. You can also inspect it with any SQLite tool.

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
at start and every 6 hours, in the background, over HTTPS (no account or token).

1. When a newer version exists, the top bar shows **Update X available**.
2. Click it (or **☰ Hosts → Software update…**) to see the version and release notes.
3. **Update now** downloads the zip for your platform, verifies it against the release's
   `.sha256` and replaces `telepad.exe` and `telepad-check.exe`. If anything fails,
   nothing is replaced.
4. **Restart now** (or **Restart for X** in the top bar) starts the new version.

**Later** hides the button until the next start; **Check now** checks immediately.
Your `telepad.db`, `config` and `.env` are never touched. Under **Update source** you
can turn the checks off or point them at a fork's repository.

## Troubleshooting

| Message | What to check |
|---|---|
| `authentication failed` | The key is not accepted or the host wants a password. Try `ssh <alias>` in a terminal; check IdentityFile and the server's `authorized_keys`. |
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
| Database | `--db <path>`, `TELEPAD_DB`, else `telepad.db` next to `config`, else next to the exe |
| First-run import | `--config <path>`, `TELEPAD_CONFIG`, `./config`, or `config` next to the exe; only read while the database is empty |
| Generated ssh config | `telepad.ssh_config` next to the database (do not edit: rewritten on every change) |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE=` in a `.env` file next to `config` |
| ssh binary | `TELEPAD_SSH` (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh` on `PATH`) |

Security model: see [SECURITY.md](../SECURITY.md).
