# add-bot-rs

trashety trash Telegram bot

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
