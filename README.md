# add-bot-rs

trashety trash Telegram bot

## Help, version, and failures

`/help` (also `/start` and `/info`) shows a short grouped overview. `/help all`
retains detailed commands, map rankings, targeting rules, form themes, and
examples. `/version` (also `/v`) returns the package version and build commit.
CI embeds the deployed commit in the binary; local builds use Git when available.
Source archives without Git can set `ADD_BOT_COMMIT`; otherwise the commit is
reported as unknown.

Recognized commands with invalid arguments explain the problem and provide a
valid example. Malformed targets are rejected instead of falling back to another
scope. Commands explicitly addressed to another bot are ignored.

Data failures distinguish unlinked players, private profiles, unavailable or
empty histories, and upstream connection/status/response failures. Chart
failures produce a reply. Available charts with missing player histories show a
partial-history notice inside the image; entirely failed lookups retain the
actual failure cause. Internal diagnostics stay in logs. Weather configuration
errors ask an admin to set a location.

## Queue reliability and delivery

Queue and preference changes operate on the latest state under one lock. State
is written to a temporary file in the same directory, synchronized, then
atomically renamed over `state.json`; the directory is synchronized afterwards.
Failed saves before replacement roll back the change. A failure after replacement
reports that the change was saved but durability could not be confirmed.
This supports one bot process using a writable state directory, including the
homelab's single-replica `Recreate` deployment.

A missing state file starts empty. Unreadable or malformed files and unsupported
future schema versions stop startup and leave the existing file untouched;
restore or repair that file before restarting. Existing files migrate in place,
preserving players and icon preferences. Legacy timed queues without dates use
their next local occurrence; legacy instant queues receive 30 minutes from
migration because their original creation date is unavailable. Legacy queues
cannot recover an original forum topic that was never stored.

New queues store absolute UTC deadlines and the originating topic. Instant
queues expire 30 minutes after creation; joins do not extend a queue's deadline.
Timed queues use the next occurrence in the configured local calendar. During
the autumn DST overlap, the first occurrence still in the future is chosen.
Nonexistent spring times advance to the first valid minute. Overdue queues are
processed after restarts or delayed polling instead of depending on an exact
minute match. Recovery more than five minutes late announces expiry rather than
claiming it is time to start playing.

Queue acknowledgements are saved with each mutation and sent before historical
analytics; prediction transitions are then added by editing the same message.
Expiry notifications are saved in the same transaction that removes the queue.
Pending notifications survive restart, respect Telegram rate-limit delays, and
use capped backoff for transient failures. Queue expiry processing runs
independently of slow Telegram requests. Text, charts, and errors preserve the
originating topic and reply context; a deleted command message does not prevent
delivery to its topic. Ordinary replies use bounded retries and return errors.

Permanent delivery failures remain in `pending_notifications` with `blocked`
set and a diagnostic in `last_failure`; they are not repeatedly retried. After
fixing permissions or the destination, an admin can stop the bot, correct the
entry, set `blocked` to false and `next_attempt` to a past timestamp, then restart.
Removing an entry explicitly abandons that notification. Delayed notifications
are labelled as such. Saved receipts prevent ordinary repeat delivery, but
Telegram has no idempotency key: an ambiguous response or a crash after Telegram
accepts a message and before its receipt is saved can still cause a duplicate.

CI requires check, tests, formatting, and Clippy (including test code) before
building the image. Pull requests build without registry login, publishing, or
deployment. Main deployment dispatches the image digest and checked source
commit; the binary and image labels also identify that commit.

## Queue and player output

Queue updates use compact headers with active occupancy, separate reserve lists,
and the existing `Predicted winrate:` comparison. `/ls` lists queues in blocks
with Today/Tomorrow labels. Instant queues are labelled “Instant queue”; ready
announcements mention the active five, while reserves remain unmentioned.

`/stats` separates recent match results from Leetify profile ratings. `/form`
shows the actual number of available results, newest first. Both retain the
requested icons and row width, with the legend below the grid and summary.
Win percentages exclude ties; tie-only histories show `—` and empty histories
have an explicit message.

Teammate records show up to five configured players, ordered by games together
within the target player's latest 30 available matches. Full rosters verify
same-team membership, so opponents are excluded. Ties count towards games played
and appear in the record only when present. Single-game records retain the
result and sample size without a percentage. Incomplete required match details
make teammate records unavailable rather than silently undercounting games.

All raster charts share the corrected glyph layout used by `/results`, including
activity/electricity captions, axes, legends, and annotations.
They also share a light background, rounded white card, typography, and muted
labels, with larger fonts for embedded Telegram previews. `/activity` defaults
to the segmented daily bar chart; `/activity @username` selects a player.
Its participant segments remain equal shares of each day's total; the legend
shows actual recorded counts separately. `/activity bars` remains an explicit
alias for the default style.

Use `/activity calendar` or `/activity @username calendar` for the calendar
heatmap (either argument order works). Shared matches count once in the
calendar; individual totals overlap. Targeted activity shows that player's
matches and counts shared matches with other configured players, without
claiming they were on the same team.

Electricity bars use the original fixed Viridis price scale: cheaper intervals
are yellow/green, while expensive intervals become blue/purple.

Results and both activity styles cover exactly 90 calendar dates, including
today, using the bot's `--tz` timezone (for example `--tz Europe/Helsinki`; the
default remains UTC). Charts show their scope, timezone, and available-history
limitation inside the image. They default to all configured players; add
`@username` for an individual view. Global results omit shared matches where
configured players faced each other, including tied games verified from team
rosters. Shared ties with unavailable rosters are omitted with an explanation.
Individual results include that player's opposing-player matches normally.

Rank and metric leaderboards use compact name-first rows and group summaries
covering all successfully fetched players. Premier ratings use thousands
separators; map and Wingman ranks show names and a named median without numeric
tier codes. Numeric medians average the middle two values for even-sized groups;
named-rank medians use the lower of the two middle ranks. Coverage footers
distinguish unavailable profiles, unranked players, and unusable flash samples
from valid zero values; summaries include only eligible entries.

`/teamflash` shows teammate hits and flashes thrown per 100 rounds, plus hits per
flash, in each row and the group-average footer.

`/predict` retains `Predicted winrate:` and explains that the estimate weights
lineup overlap. Each queue shows history coverage and unique match count; fewer
than 10 unique decisive matches gets a small-sample label. Ties count towards
the sample but not the percentage. When queued players faced each other, team
results are counted separately while the unique-match count counts the match
once. Queue join/leave messages retain their existing before/after comparison.

`/lastplayed` shows the last verified squad match's result, map, local date/time,
and configured teammates. `/hallofshame` measures days since that match using
local calendar dates, retains short dates, and shows active consecutive-day
squad streaks inline. Both verify full same-team rosters rather than treating
shared match IDs as evidence of playing together. Unverifiable histories appear
as unknown and are excluded from the inactivity average. These commands describe
the available Leetify history, which may not contain an older squad match.

## Recent form themes

Use `/form halloween`, `/stats burger 5`, or `/form @username panda` to choose
icons for recent wins, losses, and ties. Row width can be 5 or 10. Choosing a
theme for your own stats or form saves it as your preference; requests without
a theme use the target player's saved preference.

The existing themes are `squares` (initial default), `letters`, `trophy`,
`drama`, `mood`, `moon`, and `xmas`. New themes:

| Theme | Win | Loss | Tie |
| --- | --- | --- | --- |
| `halloween` | 🎃 | 👻 | 🍬 |
| `burger` | 🍔 | 🥬 | 🍟 |
| `panda` | 🐼 | 🐻 | 🎋 |
| `noodle` | 🍜 | 🫗 | 🥢 |
| `pirate` | 💰 | ☠️ | ⚓ |
| `space` | 🚀 | ☄️ | 🛸 |
| `cat` | 😸 | 😿 | 😼 |
| `dog` | 🦴 | 💩 | 🐕 |
| `weather` | ☀️ | ⛈️ | ☁️ |
| `garden` | 🌻 | 🥀 | 🌱 |
| `arcade` | 👾 | 💥 | 🕹️ |
| `slop` | 🤖 | 🗑️ | 🫠 |
| `hotdog` | 🌭 | 💩 | 🥖 |
| `stocks` | 📈 | 📉 | ➖ |
| `team` | 🏆 | 🚑 | 🤝 |
| `random` | Random | Random | Random |
| `counterstrike` / `cs2` / `kynäri` | 💣 | 🐔 | 🛡️ |
| `mistakes` | 🎯 | 🤦 | 🤷 |
| `bike` | 🚴 | 💥 | 🚲 |
| `car` | 🏎️ | 🚧 | 🚗 |
| `traffic` | 🟢 | 🔴 | 🟡 |

`random` draws one emoji from each of three curated pools on every invocation:
60 good/win emoji, 60 bad/loss emoji, and 60 neutral/tie emoji (180 total).
Each emoji in its pool has an equal chance of being drawn. The pools do not
overlap, so the three emoji are distinct. The win/loss/tie mapping stays
consistent within that response and is shown in the legend.
You can save `random` as your preference just like any other theme.

Custom themes use three emoji in **win / loss / tie** order instead of a theme
name: `/form 🍟🥬➖` or `/stats 🍟🥬➖ 5`. Spaces between the emoji also work,
as does a target username: `/form @username 🍟 🥬 ➖ 10`.
Flags, skin tones, and joined emoji count as one emoji each. Exactly three emoji
are required; repeated emoji are allowed. Choosing a custom theme for yourself
saves it for future `/form` and `/stats` requests, including after a restart.
Choose a named theme again (for example, `/form squares`) to replace it.

## Match results

`/results` shows the configured players' unique matches over the last 90 UTC
calendar days. `/results @username` shows that player's matches instead.

The chart includes win/loss/tie totals, a rolling 20-match win rate, and one
green/red/yellow square per match, aligned by date. Matches within each day run
from top to bottom. The strip grows vertically to show busy days without hiding
results; blank days mean no games.

The trend uses one point per played day, calculated after that day's final match
and centred over its date column. It still considers all matches in the rolling
window; multiple games within a few hours no longer produce narrow spikes.

Both percentages exclude ties (`wins / (wins + losses)`). Ties still occupy a
place in the 20-match window and remain visible in the totals and strip. The
trend starts once 20 known matches are available; available matches from before
the displayed period can prime that window. A window containing only ties has no
percentage. Gaps of more than three days use a dotted connector with no shaded
area. No matches, short histories, and tie-only histories have explicit messages.

## Electricity forecast

`/el` sends one Porssisahko price image with current/low/high prices, quieter
grid lines, and a current-time marker. When the chat has active CS2 queues,
their electricity-cost estimates appear in a panel beneath the plot, labelled
per PC per estimated match. No separate forecast message is sent. The panel
shows the PC load, estimated duration and sample, included variable charges,
and unavailable estimates. Its height grows with the number of queues.

- An instant queue is priced as if the match starts now, because its actual
  fill time is unknown.
- A timed queue is priced from its next scheduled occurrence.
- Match duration is estimated from the newest 30 completed Leetify matches
  with usable `rounds_count` data. Leetify does not expose duration, so the
  estimate uses 2 minutes per round.
- The default load is 500 W for the gaming PC.
- Porssisahko prices are used in c/kWh as returned by the service; its current
  frontend identifies them as including VAT.
- The default Caruna Espoo Yleissiirto variable charge is 2.77 c/kWh including
  VAT, from the tariff effective 2026-01-01. The fixed monthly transfer fee is
  excluded because it cannot be attributed to one match.
- A separate electricity tax is not included in this estimate; the model is
  intentionally limited to spot energy plus the requested variable transfer
  charge.

The forecast is calculated as:

```text
energy (kWh) = PC power (kW) × estimated match duration (hours)
spot cost = time-weighted Porssisahko price × energy
Caruna cost = 2.77 c/kWh × energy
forecast total = spot cost + Caruna cost
```

The power and Caruna assumptions can be overridden in `Settings.toml`:

```toml
[electricity]
gaming_pc_power_watts = 500.0
caruna_espoo_distribution_cents_per_kwh = 2.77
```

Sources:

- [Porssisahko](https://porssisahko.net/)
- [Caruna Espoo network service tariff 1 January 2026](https://caruna.fi/tuotteet-ja-palvelut/kotiin-ja-kiinteistoon/verkkopalveluhinnastot/verkkopalveluhinnasto-caruna-0)

## Weather output

`/weather` shows temperature, wind/gusts, the next 1/6-hour conditions and
precipitation, and tomorrow's forecast high. Humidity, cloud cover, and pressure
are omitted from the compact report. Conditions and precipitation come from
the same forecast period; unavailable future temperatures are shown as `N/A`.
Both `/weather` and `/temperature` calculate tomorrow using the chat's local
calendar date, including daylight-saving changes.
