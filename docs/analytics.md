# Anonymous usage statistics

DegenView counts how many people run it and in which country. It is **on by default** and can be
turned off in Settings ▸ About ▸ Usage Statistics. Until this feature, the app sent nothing about its
users anywhere.

## What is sent

One HTTPS POST per launch, about 20 seconds after start, to Google Analytics 4's Measurement Protocol
(`https://www.google-analytics.com/mp/collect`). The body is built by `service::analytics::payload`:

| Field | Value |
| --- | --- |
| `client_id` | A random 128-bit id made on this install (`store::sync::new_device_id`). Not derived from the Mac, the account or iCloud. |
| `app_open` | The event, with the parameters below. |
| `new_install` | Sent once before `app_open` until Google has accepted it once. GA reserves `first_open`, so this is our own name. |
| `app_version` | `CARGO_PKG_VERSION`, e.g. `0.6.0-rc.17`. |
| `os`, `arch` | `std::env::consts`, e.g. `macos` / `aarch64`. |
| `update_channel` | `stable` or `rc`: the channel chosen in Settings ▸ About. |
| `packaging` | `flatpak` or `native`. Nothing finer is knowable: the dmg, the zip and Homebrew install the same app. |
| `engagement_time_msec` | `1`. GA4 drops Measurement Protocol events without it from its reports. |

**Country** comes from the IP address of the request. Google works it out on its side; DegenView sends
no location and does not read the address.

**Never sent**: the build id, symbols, charts, watchlists, portfolios, scripts, drawings, alerts,
keys or tokens, paths, the machine's name, the account name, or any text the user typed. A test pins
the exact set of parameter names, so adding one is a visible change that has to update this page.

## When nothing is sent

- The user turned **Usage Statistics** off, or answered **Turn Off** on the first-run notice.
- The user has **not yet seen the notice**. The first launch of a build that counts shows a notice
  (`show_analytics_notice`, in the focused window only); nothing is sent until it is answered, so a
  user who quits first is not counted. An existing install meets the same notice once.
- `DEGENVIEW_NO_ANALYTICS` or `DO_NOT_TRACK` is set to anything but empty or `0`.
- The build is a debug or test build (`cfg!(debug_assertions)`). A developer's runs are not users.
- The build has no GA ids (below). Then Settings shows no switch and no notice appears: there is
  nothing to turn off.
- The network fails or is slow: the post has a 5 second timeout and no retry within a launch, and
  its failure is silent. A failed `new_install` is announced again next launch.

Turning counting off **forgets the install id** and that it was counted. Turning it back on later makes
a new anonymous id, so the two stretches cannot be joined.

## Where it is stored

Four local `setting` keys, none of them in `sync::policy` (so none syncs, and a test pins that). An id
that synced would merge every Mac of one person into one user and would sit in iCloud.

| Key | Meaning |
| --- | --- |
| `analytics.enabled` | The user's choice. Absent means on. |
| `analytics.noticeShown` | The notice was answered, or the user chose in Settings. |
| `analytics.clientId` | The random install id. Removed when counting is turned off. |
| `analytics.installCounted` | Google accepted `new_install`. |

A copied database (Migration Assistant, a restored backup) carries the id with it, so two machines can
report as one user. That is accepted: the count is approximate.

## Build-time ids

`DEGENVIEW_GA_MEASUREMENT_ID` (`G-…`) and `DEGENVIEW_GA_API_SECRET` are read with `option_env!`. A
build without both, or with a value that is not letters, digits, `-` and `_`, has no config and sends
nothing. `release.yml` passes them from the repository **variables** of the same names to the macOS,
Windows and Linux compile steps. They are variables, not secrets, because they ship inside the
binary: anyone can read them out of a release and post fake events (a GPL app has no way around
that), so treat the numbers as approximate. A local test build:

```bash
DEGENVIEW_GA_MEASUREMENT_ID=G-XXXXXXXXXX DEGENVIEW_GA_API_SECRET=... cargo run --release
```

The Flatpak is built in a sandbox (`flatpak.yml`) that does not pass these variables, so **Flatpak
builds send nothing** until they are threaded through its manifest. Homebrew installs the release zip,
so it counts.

## Setting up the Google side

1. analytics.google.com ▸ Admin ▸ Create ▸ **Property** (GA4).
2. Admin ▸ Data streams ▸ Add stream ▸ **Web** (the URL is not used; the stream only issues the
   Measurement ID `G-…`).
3. In that stream: **Measurement Protocol API secrets** ▸ Create. Copy the secret.
4. GitHub ▸ degenview/degenview ▸ Settings ▸ Secrets and variables ▸ Actions ▸ **Variables**: add
   `DEGENVIEW_GA_MEASUREMENT_ID` and `DEGENVIEW_GA_API_SECRET`.
5. To break reports down by `os`, `arch`, `app_version`, `update_channel` or `packaging`, register each
   as an event-scoped **custom dimension** (Admin ▸ Custom definitions). Country, users and new users
   need nothing.
6. Check a payload without recording it: POST it to
   `https://www.google-analytics.com/debug/mp/collect?measurement_id=…&api_secret=…`; the reply lists
   `validationMessages` (empty means valid). Then look at Reports ▸ Realtime after a launch.

## Apple's analytics, and why not

- **App Store Connect ▸ App Analytics** counts App Store and TestFlight installs. DegenView is
  distributed outside the App Store (Developer ID), so it would report almost nothing, and it covers
  macOS only.
- **MetricKit / Xcode Organizer** give crash and performance reports, not users or origin.

If Google Analytics is ever rejected, the replacement is a privacy-first service (TelemetryDeck,
Plausible), not an Apple product. Only `service::analytics` and the two build variables would change.

## Not done

- Campaign or referrer attribution (UTM): a desktop app has no referrer, and nothing here controls a
  download page. GitHub's per-asset `download_count` already splits dmg, zip, installer, tarball and
  Flatpak downloads, but counts downloads, not users.
- Install source finer than `flatpak` / `native`.
- Feature-usage events, crash reports.

## Verified and not verified

- Verified by unit tests: the exact payload keys, the `new_install` / `app_open` sequence, opt-out by
  switch, by environment and by an unanswered notice, id creation, persistence and removal, that none
  of the keys syncs, the POST's method, header and body against a local server, and that Settings
  draws with and without the row.
- **Not verified**: a request reaching Google (no property exists yet), the `debug/mp/collect`
  validation, `new_install` and `app_open` showing in Realtime, country appearing, the release workflow
  passing the variables, the notice and the switch in a real window, any Windows or Linux build.
