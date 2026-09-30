# add-bot-rs

trashety trash Telegram bot

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

Rank and metric leaderboards use compact name-first rows and group summaries
covering all successfully fetched players. Premier ratings use thousands
separators; map and Wingman ranks show names and a named median without numeric
tier codes. Numeric medians average the middle two values for even-sized groups;
named-rank medians use the lower of the two middle ranks.

`/teamflash` shows teammate hits and flashes thrown per 100 rounds, plus hits per
flash, in each row and the group-average footer.

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

`/el` always sends the existing Porssisahko price chart. When the chat has
active CS2 queues, it also sends an electricity-cost forecast for each queue.

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
