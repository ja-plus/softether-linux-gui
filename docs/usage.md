# SoftEther Tray Usage Guide

**English** | [简体中文](usage.zh-CN.md)

## Starting the app

- Open **SoftEther Tray** from the application menu, or run `softether-tray` directly
- Closing the window does not quit the app — it only hides to the tray. To really quit, use **Quit SoftEther Tray** in the tray menu
- Left-click the tray icon = open the main window; right-click = menu (status, connect/disconnect, network diagnostics, quit)

## Connecting

1. On the "Connect" tab click **Connect** on a profile, or click **Connect xxx** directly in the tray menu
2. The first privileged operation may pop up a system password prompt (pkexec), used to start the service and install routes; **at most one prompt per connection**
3. Once connected, the status card shows the tunnel IP, MTU, total traffic, and live rates

On disconnect the app removes the subnet routes and clears the adapter address, so no stale route is left turning the intranet into a black hole.

### When the password prompt appears

Elevation happens **only when the kernel network configuration actually has to change** — not on every action:

| Action | Password prompt? | Why |
|---|---|---|
| Connect | Yes (**once only**) | Needs to obtain a DHCP address, change the MTU, and install subnet routes |
| Disconnect | **Usually not** | Tearing down the tunnel itself needs no root; elevation only happens to clean up leftover routes or IPv4 on the adapter |
| Start/stop the client | Yes | `vpnclient` is a root daemon |

A single connection really needs root for three things: requesting a DHCP address, changing the MTU, and installing subnet routes. These three are folded into **one script and one pkexec call**, so the password prompt appears only once. The gateway address is also computed on the fly by the shell inside that script (the first hop of the adapter's `proto kernel` route) instead of being read through a separate elevation — that would be a second prompt.

If the sudoers whitelist (see the end of this doc) is installed, client start/stop calls pass silently through `sudo -n`, and connecting may show **no password prompt at all**.

Disconnect works in the order "tear down the tunnel first, clean up afterwards". If the cleanup step fails (e.g. you cancel the password prompt) the disconnect itself still succeeds — the tunnel is already down, but routes may linger; the UI will tell you to clean them up manually:

```bash
sudo ip route del 10.0.0.0/8 dev vpn_vpn
```

Only delete subnets this app installed. IPv6 routes such as `fe80::/64`, local, and multicast are maintained by the kernel along with the adapter; they need not and should not be deleted by hand.

### The tunnel is up but the adapter has no IP

The SoftEther Linux client only establishes a link-layer tunnel; **it does not configure an IP on the virtual adapter for you**. So "session established" and "adapter has an address" are two different things — without the latter there is nowhere to install routes, and the intranet is simply unreachable.

The app's connection flow is:

1. Build the tunnel and wait for the session state to become "connected"
2. Check whether the virtual adapter has an IPv4 address; **if not, run DHCP once automatically** (`dhclient -1` or `dhcpcd -1` — it makes one request and exits, leaving no daemon inside the tray app)
3. Only if there is still no address does it report an error, with two different messages depending on the cause

Since DHCP is run automatically, an error usually means one of two things:

| Message | Meaning | What to do |
|---|---|---|
| DHCP got no response | The local client is fine, but the server hands out no address | Enable the "Virtual DHCP Server" on the server-side HUB, or configure a static address on the adapter |
| No dhclient/dhcpcd on the system | The machine lacks a DHCP client | `sudo apt install isc-dhcp-client`, or configure a static address manually |

**Configure a static address** (substitute your actual subnet):

```bash
sudo ip -4 addr add 192.168.50.19/24 dev vpn_vpn
sudo ip link set dev vpn_vpn up
```

After that, click "Connect" again — the app detects the address and installs the routes.

### Why you have to fill in the subnet list yourself (and what the button verifies)

This is the biggest experience gap between the Linux and Windows clients, and the cause has been identified — it is not a configuration problem:

**The Windows client writes the routing table itself; the Linux client does not.**

The Windows `vpnclient` runs as LocalSystem and writes routes pushed by the server's virtual DHCP (`PUSHROUTE`) straight into the system routing table, invisibly to the user. The Linux client's service **contains the same code compiled in**, but in practice does not do this — after reconnecting, the routing table shows no new entries at all, and no DHCP client is started either. So intranet subnets can only be written by this app on your behalf.

Conclusion: **you cannot ask the local side what the server pushed**. Hence the list is yours to fill in — or at least to confirm the three defaults are enough.

### What the "Verify subnets" button tests

It only answers questions it can answer **reliably** — it does no auto-discovery:

| Result | Meaning | What to do |
|---|---|---|
| `OK` | Traffic for that subnet really goes into the VPN adapter, and the far side of the tunnel responds | Nothing to do |
| `NOROUTE` | Traffic for that subnet **does not go through the VPN**; the physical adapter picks it up | Check that the subnet was saved, then reconnect |
| `DEAD` | The route exists, but the tunnel as a whole is down | Check the connection state and whether the adapter has an IP first |
| `TUNNEL down` | The gateway does not answer pings; the tunnel is down | Reconnect |

**Why no auto-discovery**: two guessing strategies were tried and both were disproven by real testing —

1. Assume the subnet's `.1`/`.254` is the gateway → `10.0.0.1` does not exist at all, yet `10.10.4.160` answers (27–40 ms), so the intranet device is at neither `.1` nor `.254`;
2. Read the ARP table → it only shows neighbors inside the same broadcast domain (`192.168.50.0/24`); targets on other subnets never appear there.

Likewise, **it draws no conclusion about "whether an IP in a given subnet is reachable"**: within one subnet some machines answer ICMP and some do not, so letting a single address stand for a whole subnet guarantees false reports. The button therefore vouches only for the hard fact of "is the route in effect".

### A real case: the defaults are not enough

If the intranet address you need is not covered by the three defaults, the symptom is "the VPN says connected, but that address just won't answer", and the cause is **hard to see** — the packets quietly leave through the physical adapter:

```bash
ip route get 10.10.4.160
# before fix: 10.10.4.160 via 192.168.84.254 dev wlp0s20f3   <- Wi-Fi, 100% loss
# after fix:  10.10.4.160 via 192.168.50.1 dev vpn_vpn       <- tunnel, 27-40ms
```

The rule of thumb: in `ip route get <target>`, the `dev` must be `vpn_vpn`. Anything else means that subnet is not covered.

## Where the profiles come from

The app reads connection profiles straight out of the SoftEther client, which means:

- Accounts created earlier with `vpncmd` or imported configs show up as soon as you open the app
- Profiles created or edited here likewise appear in `vpncmd` and `vpn-tracy`
- Deleting a profile removes only that SoftEther entry; virtual network adapters are left alone

## Passwords

The password is handed to `vpncmd` only at the moment of saving, written to a temporary file with mode 0600 and deleted immediately afterwards; `settings.json` contains only routes and UI preferences — never passwords.

## Launch at login

"Launch at login" in Settings registers a desktop-session autostart entry (`~/.config/autostart`). Pair it with "Auto-connect after launch" to bring the VPN up automatically after login.

## Interface style

At the top of Settings sits "Interface style", with three looks to choose from:

| Style | Look |
|---|---|
| Style 1 · Windows 11 | Fluent light-gray canvas, square-cornered controls, accent indicator bar on the left |
| Style 2 · Deepin 25 | Large-radius cards, solid blue navigation pills, soft shadows |
| Style 3 · macOS | Traffic lights top-left (red/yellow/green), light-gray sidebar, solid blue rounded selection, very light shadows |

Clicking a style previews it across the whole page and writes `uiStyle` to `settings.json` automatically; it carries over to the next launch, and leaving the page also saves — there is no separate "Save" button. On first install the style is picked by distribution: Deepin-family gets Style 2, everything else Style 1.

## Client service lifecycle

The SoftEther Linux client is a **long-running root daemon** (two `vpnclient execsvc` processes) with no systemd unit of its own. This app can manage its start and stop:

| Setting | Effect |
|---|---|
| Start the client service when the app starts | Automatically brings up `vpnclient` when the app opens (skipped if the service is already running) |
| Stop the client service when the app exits | Stops the client when the app closes |

Checking both = **the client follows the app: up with it, down with it**.

**Only a client started by this app gets stopped on exit.** When the app actually starts the client it leaves a flag at `~/.config/softether-tray/client-managed.flag`; a `vpnclient` you started yourself in a terminal is never stopped by the app, so nobody else's connection gets cut by accident. Clicking "Stop service" manually clears the flag as well.

The flag records **"did this call actually start the client"**, not "is the client running now". The client is a self-daemonizing shared process the user may already have started; writing the flag by the latter rule would take your manually started tunnel down together with the app. So every start path (app launch / clicking Connect / clicking Start service) first checks whether the service is already running and writes nothing if it is.

Stopping the service runs "disconnect accounts → clear routes → stop the client", so it never leaves a stale route that turns the intranet into a black hole. If the app is force-killed with `kill -9`, that cleanup cannot run — just reopen the app and stop the service once.

**Authorization wait on exit is capped**: without the sudoers whitelist, stopping the client pops one password prompt. On exit the app waits at most 20 seconds, then quits leaving the client still running — quitting is an explicit user intent and should not hang on a password prompt. Clicking "Stop service" in the UI has 120 seconds, plenty of time to type a password.

"Quit" in the tray menu and "Quit app" in the UI run the same teardown logic and behave identically.

### Password-free startup (optional)

Two approaches exist; **polkit is recommended**.

#### Option A: polkit authorization (recommended, finest-grained)

```bash
sudo bash Tools/install-polkit.sh            # install and enable password-free
sudo bash Tools/install-polkit.sh --no-auto  # install the policy only, still prompts
sudo bash Tools/install-polkit.sh --remove   # remove completely
```

It does three things:

1. Fixes the privileged operations into one **argument-restricted** script `/usr/libexec/softether-tray/softether-tray-net`
2. Uses `org.freedesktop.policykit.exec.path` in `cn.softether.tray.net.policy` to **bind the action to that single path**
3. Uses `50-softether-tray.rules` to allow `YES` for that one action only

So connect, disconnect, and client start/stop all go through silently. Meanwhile `pkexec ip ...`, `pkexec bash`, `pkexec dnf ...` still demand a password — which is exactly what makes this safer than a sudoers whitelist.

Hard limits built into the script itself (read the file header before modifying it):

| Limit | What it prevents |
|---|---|
| Interface name must have the `vpn*` prefix and exist in `/sys/class/net` | Otherwise it could change `eth0`'s address |
| Subnets must be dotted-quad IPv4 with `/0..32`; `0.0.0.0/0` is rejected | Otherwise it could be used to hijack all traffic |
| MTU restricted to 576..1500 | Prevents killing the link |
| Only the three actions `connect`/`cleanup`/`vpnclient` are accepted | No arbitrary commands, no shell fragments |
| The rule requires a **local active session** + membership in `sudo`/`wheel` | Remote SSH and other people's sessions still need a password |

#### Option B: sudoers whitelist

```bash
sudo bash Tools/install-sudoers.sh          # install
sudo bash Tools/install-sudoers.sh --remove # remove
```

It opens up only the two command combinations `<client path> start` / `stop`, validated with `visudo -c` before installing. Afterwards client start/stop calls pass silently through `sudo -n`.

⚠️ But the sudoers whitelist is **command-level**: it allows "any process may run `vpnclient stop`", unlike the polkit approach which restricts the calling session too. You can install just one of the two; if both are installed, polkit takes precedence.

## Troubleshooting

**Status keeps showing "service not started"**
Click "Start service", or run `vpn-tracy` in a terminal. The app pops a password prompt when administrator rights are needed.

**Polkit is installed but the password prompt still appears**
Check in this order:

1. Confirm the helper is in place: `ls -l /usr/libexec/softether-tray/softether-tray-net` (should be 755 root:root)
2. Confirm the rule is in place: `ls -l /etc/polkit-1/rules.d/50-softether-tray.rules`
3. Confirm polkitd recognizes the action:
   ```bash
   busctl call org.freedesktop.PolicyKit1 /org/freedesktop/PolicyKit1/Authority \
     org.freedesktop.PolicyKit1.Authority EnumerateActions s "" | grep -c cn.softether.tray.net
   ```
   A `0` result means the policy was not loaded — check the policy file permissions (must be 0644, in a directory polkitd can read).
4. Confirm you are in the `sudo` group: `id -nG | tr ' ' '\n' | grep -x sudo`
5. See whether polkit rejected the rule: `journalctl -u polkit -n 50 --no-pager`
6. **Note**: if `/home/<you>` has mode `drwxr-x---` (the default on many distributions), polkitd cannot enter the home directory, so anything under `~/.config/polkit-1/` **has no effect**; it must be installed with `sudo` into system directories — which is what this installer does.

**Connected, but the intranet is unreachable**
Look at the subnet list in "Settings → Tunnel & routes" first. Only subnets in the list go through the VPN; intranet subnets not in the list must be added yourself.

**Packet loss, stutter**
Test the target address in "Network diagnostics" first, then narrow it down in this order:

1. The local gateway (the default gateway in `ip route`)
2. The VPN server's outer address (`getent ahostsv4 <server domain>`)
3. The intranet target

The layer where loss starts is where the problem is. Historical experience: a broken UDP acceleration shows up as stable ICMP, extremely slow TCP connects, and the client pegging a CPU core — the fix is to turn UDP acceleration off.
