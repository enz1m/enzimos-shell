# Search providers: apps adding results to Search

Search (Super+Space) can show results that come from inside apps: messages from
Mail, events from Calendar, notes from Notes. Each such app is a **search
provider**. Search asks every enabled provider for results as the person types.
It merges them under the app's name, and when one is chosen, it asks the app to
open it.

The model follows GNOME Shell's search providers and KDE's KRunner D-Bus plugins.
A provider is a D-Bus service the app already owns, declared by a small file. The
desktop never loads app code and never reads app data. **GNOME `SearchProvider2`
providers already installed work as well**, so Calendar, Firefox, Calculator and
others take part without changes.

## 1. Declaring a provider

The app installs one file per provider in a data directory:

```
$XDG_DATA_DIRS/enzui/search-providers/<desktop-id-without-.desktop>.ini
```

```ini
[enzUI Search Provider]
DesktopId=io.github.enzui.Mail.desktop
BusName=io.github.enzui.Mail
ObjectPath=/io/github/enzui/Mail/SearchProvider
Version=1
# Optional:
MinQueryLength=2
```

- `DesktopId` names the app. Search shows the app's own name and icon from its desktop entry, and drops the provider when that entry is missing, hidden, `NoDisplay`, or not allowed for the account by parental controls.
- `BusName` must belong to the app: for an enzUI provider it is **exactly** the desktop ID without `.desktop` (`io.github.enzui.Mail`); for a GNOME one, that ID or a name under it (`org.gnome.Epiphany.SearchProvider`). A declaration whose bus name names another app is ignored.
- A Flatpak app may only own names under its own ID. That is what ties a sandboxed provider to its app.
- Files under `$XDG_DATA_HOME` override system ones with the same name. Unknown keys are ignored. Files larger than 4 KiB, or with an invalid bus name or object path, are ignored.

GNOME providers are read from `$XDG_DATA_DIRS/gnome-shell/search-providers/*.ini` (`[Shell Search Provider]`, `Version=2`; `DefaultDisabled=true` ones are skipped). An enzUI provider for the same `DesktopId` wins.

**GNOME providers are off until the person turns them on** in Settings › Search.
Providers declared for enzUI are on by default. Enabled providers are queried with D-Bus auto-start explicitly suppressed.
When the OS advertises `search_policy_adapter`, an installed enzUI app can
start through Background1 only if its separate search activation source is
allowed in Settings › Apps › Background apps. Without that capability or
permission it is queried only while running. GNOME providers are always
queried only while running. Enabling a provider is not a user-launch or MCP grant.

## 2. The interface: `dev.enzui.SearchProvider1`

All calls go to `BusName` at `ObjectPath`.

| Member | Signature | Purpose |
|---|---|---|
| `Search` | `(s query, a{sv} options) → aa{sv} results` | Results for one query. |
| `Activate` | `(s id, s query, a{sv} options) → ()` | Open the chosen result in the app. |
| `LaunchSearch` | `(s query, a{sv} options) → ()` | Open the app's own search for the query ("Show all in Mail"). |
| `Version` (property) | `u` | `1`. |

**`Search` options**

| Key | Type | Meaning |
|---|---|---|
| `limit` | `u` | The most results Search will show from this provider (at most 20). Sending more is wasted. |
| `locale` | `s` | The desktop's language, e.g. `en_IE`. |

**A result** is a dictionary:

| Key | Type | Required | Bound |
|---|---|---|---|
| `id` | `s` | yes | ≤ 256 bytes; opaque to Search, given back to `Activate` |
| `title` | `s` | yes | ≤ 200 characters shown |
| `subtitle` | `s` | no | ≤ 300 characters shown, one line |
| `icon` | `s` | no | a themed icon name; the app's icon otherwise |
| `score` | `d` | no | 0.0–1.0, the provider's own relevance, for ordering within its group |

Results without an `id` or `title`, with control characters other than tabs
and line breaks in the title, or beyond the limit are dropped. Tabs and line
breaks become spaces. Text is shown as plain text on one line, never as
markup.

**`Activate` and `LaunchSearch` options:** `activation-token` (`s`), when the desktop has one: an XDG activation token the app passes to the window it raises, so focus goes to the app (see `xdg-activation-v1`). Search requests a fresh token from the window where the result was chosen and passes it to enzUI providers. Apps must still work without one when the compositor cannot provide it; receiving a token does not guarantee focus will be granted. GNOME's legacy provider interface has no token argument and retains its timestamp framing. `timestamp` (`u`) is included for GNOME-style apps.

## 3. What Search does

- **Separate permission to start an app.** `Search` calls always carry D-Bus's no-auto-start flag. An enabled enzUI provider that is not running may be started once through `Background1.ActivateForSearch` when the OS advertises `search_policy_adapter`. The OS verifies installed declarations and unit identity and requires the app's separate `search` policy. Missing capability, denial, timeout or an unsupported provider leaves the app unavailable. A query does not count as a user launch and grants no MCP access. The provider answers without a window; choosing a result calls `Activate` or `LaunchSearch` with a fresh activation token when available. GNOME providers are queried only while running.
- **When it asks:**
  - only after the query has at least `MinQueryLength` characters (default 2);
  - only while the "In apps" source is enabled in Settings › Search;
  - only for providers the person hasn't switched off;
  - not while a category filter other than "In apps" is chosen.
- **Concurrency:**
  - every enabled provider is asked at once, about 120 ms after typing pauses;
  - a provider's answer counts only if the query is still current;
  - each query call has a 1.5 s deadline;
  - results that arrive later than 400 ms are merged in place without moving the selected row;
  - a provider that times out three times in a row is skipped for the rest of the session and shown as unresponsive in Settings.
- **Merging:**
  - provider groups follow the built-in results (apps, windows, actions);
  - each group is headed by the app's name and shows at most 5 results, in the provider's order (or by `score`);
  - the last row of a group, "Search in Mail", calls `LaunchSearch`.
- **Activation:**
  - `Activate` or `LaunchSearch` with a fresh activation token when available;
  - a late token reply is discarded if the search window/session or selected result generation changed;
  - Search closes after a successful call; explicit opening allows up to 10 s for a cold app to start;
  - an enzUI app's version property must become available before the one open call is sent; only this read-only readiness check can repeat during startup;
  - an open failure is shown in Search's status line; the open call is never retried.
- **Privacy:**
  - typed **or pasted** text goes to every enabled, running provider; Search can't tell whether pasted text was a password, and Settings says so;
  - the query goes only to apps the person has installed and left enabled;
  - providers get no agent or MCP authority: a query is not a grant, and an app must not treat it as one;
  - Settings › Search lists every provider with a switch: enzUI providers start on, GNOME ones off;
  - queries are at most 256 characters, Search's own limit.

## 4. What a provider must do

- **No window on `Search`.** Answering must not map a window, raise one, steal focus or show a notification. Only `Activate` and `LaunchSearch` may bring up the app's UI, in the app's single running instance.
- **One instance.** If the app is already running, the provider is that instance; a second activation must not start another copy.
- **Cheap answers.** `Search` is called on every pause in typing; answer from an index, within a few hundred milliseconds. Late answers are dropped.

## 5. Writing a provider in an enzUI app

The app side uses the pinned framework's search-provider helper. It serves `dev.enzui.SearchProvider1` from the app's existing session-bus connection. It turns `Activate` and `LaunchSearch` into app messages through the same wake/drain route as the app's other runtime events. It also writes the `.ini` file at packaging time. The app supplies a function from a query to results over its own index, for example a mail store's full-text index. Searching must be cheap: Search calls it on every pause in typing.

## 6. Testing a provider

`enzui-search-probe <declaration.ini> [query…]` (a shell binary) does the following against the running session bus:

- reads the declaration and names the app it belongs to;
- checks that the bus name is running or activatable (Search itself only asks running apps, so start the app before probing);
- runs `Search` for each query, timing it and applying every bound above;
- prints what Search would show.

`enzui-search-demo-provider` is a sample provider with made-up messages.
`enzui-search-demo-provider --install DIR` writes its declaration, D-Bus service and desktop entry under the data directory `DIR`. It records what it is asked to open in `$XDG_RUNTIME_DIR/enzui-search-demo.log`.

Inside a nested preview, providers come from the preview's own data directories and private session bus. Install into the preview profile's `client-data` directory, which is the preview's `XDG_DATA_HOME`.
