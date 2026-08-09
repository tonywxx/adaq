# Paper Trading Accounts and Portfolios

[简体中文](./paper-trading-accounts.zh-CN.md)

Status: V1 user and evidence contract.

Related guides: [Paper Provider Connections and Credentials](./paper-connections.md), [Trading Bot Runtime](./bot-runtime.md), and [Strategy, Risk, and Execution](./strategy-risk-execution.md).

## One account, one currency, one ledger

Each Paper Portfolio belongs to exactly one Paper Trading Account and one Valuation Currency. A Portfolio may coordinate many Instruments available through that Account, but it cannot borrow from or share cash with another Account.

| Market | Paper Execution Adapter | Authoritative ledger | Valuation Currency | Funding target |
| --- | --- | --- | --- | ---: |
| China A-shares, Ordinary Securities Account | `adaq-a-share-paper` | ADAQ-owned simulator | CNY | 1,000,000 CNY |
| U.S. equities | `adaq-alpaca-paper` | Alpaca Paper | USD | 1,000,000 USD |
| Crypto Spot | `adaq-okx-paper` | OKX Demo Trading | USDT | 1,000,000 USDT |

All balances, quantities, prices, fees, and PnL use exact decimal representations. The funding target is not permission to falsify an external provider's balance.

## External versus ADAQ-owned accounts

For an ADAQ-owned simulator, the funding target initializes a new paper ledger according to its explicit reset workflow.

For Alpaca or another external Paper Provider, the latest validated Paper Account Snapshot is authoritative. If the provider reports USD 100,000 while the desired target is USD 1,000,000, ADAQ shows both values and the discrepancy. It does not display USD 1,000,000 as available cash and does not silently issue a remote reset.

Credentials are never part of a Paper Account Snapshot, Run export, log, screenshot requirement, or Component input. Configure them only through the host-owned workflow in [Paper Provider Connections and Credentials](./paper-connections.md).

## Paper Execution Adapters

All three Adapters share ADAQ's internal order, Fill, Account Snapshot, error, and reconciliation evidence contract, but provider behavior is never flattened into a false lowest-common-denominator simulation.

- **OKX Demo Trading** uses a simulated-account API key and simulated-trading request mode. OKX order IDs, states, errors, account events, and Fills remain provider-authoritative.
- **Alpaca Paper** uses the Paper endpoint and Paper account credentials. Alpaca acknowledgements, rejections, partial Fills, buying power, and Account Snapshots remain provider-authoritative.
- **A-share Paper** is an ADAQ-owned Ordinary Securities Account simulator because V1 has no selected free external A-share Paper API. Its append-only local ledger and exact market-rule inputs are authoritative. A Credit Account, financing, securities lending, short selling, and margin liabilities are unavailable.

Each Account freezes a Paper Execution Capability Snapshot covering supported Instruments, order types, sessions, extended hours, precision, buying power, margin or short behavior, fill assumptions, event streams, rate limits, and account-reset capabilities. The UI must show unavailable capabilities instead of offering controls that the selected Adapter cannot honor.

See [OKX API FAQ](https://www.okx.com/help/api-faq) and [Alpaca Paper Trading](https://docs.alpaca.markets/us/docs/paper-trading) for the provider-owned Paper environments.

## Reconciliation after startup or disconnection

ADAQ must not continue from a stale cached account after a network interruption. At startup, reconnect, account-event sequence gap, or detected mismatch it:

1. Blocks new risk for the affected Account.
2. Retrieves a provider-authoritative Account Snapshot and open-order state.
3. Replays or resumes ordered account, order, and Fill events when the provider supports them.
4. Compares cash, buying power, positions, reserved funds, orders, Fills, and event identities with the local evidence ledger.
5. Records every explained correction and unresolved difference.
6. Resumes only after the Account reaches a reconciled state under the frozen policy.

An unresolved mismatch never becomes “fixed” by overwriting the provider or deleting local history. The Dashboard must show the affected Account as unreconciled and explain which values disagree.

## Portfolio binding

A Strategy Instance binds one Paper Portfolio before Paper Trading starts. That binding freezes:

- Paper Trading Account identity and provider.
- Valuation Currency.
- Eligible Venue and Instrument Universe.
- Starting Paper Account Snapshot.
- Strategy allocation within available account capital.
- Risk Policy and Execution Profile.
- Calendar, settlement, precision, fee, and market-rule evidence.

A Portfolio Strategy may rank and allocate many U.S. equities inside Alpaca Paper, many A-shares inside the ADAQ simulator, or many Spot Instruments inside the Crypto Paper Account. It cannot emit one Portfolio Target containing `600519`, `AAPL`, and `BTC-USDT` in V1.

## Dashboard presentation

The Dashboard must show each account independently:

```text
A-share Paper     1,000,000 CNY target     observed CNY equity
Alpaca Paper      1,000,000 USD target     observed USD equity
Crypto Paper      1,000,000 USDT target    observed USDT equity
```

ADAQ must not add these three numbers into one “total equity.” CNY, USD, and USDT are different economic units. Account cards may show their native equity, cash, buying power, reserved capital, exposure, PnL, drawdown, connection state, and active Bots side by side.

If a future view presents a converted global total, it must display the reporting currency and exact FX Snapshot, source, time, rate, and unavailable conversions. V1 has no such global total.

## Multiple Bots

Several Bots may use one Paper Trading Account only under explicit capital reservations and one host-owned account risk boundary. Their Strategy Allocations must not exceed available capital, and every open order must reserve funds before another Bot calculates buying power.

Bots on different Paper Accounts remain operationally independent. A network or broker failure on one Account does not alter another Account's ledger, though a global operator action such as Freeze All may intentionally pause all of them.

## Evidence users must be able to inspect

- Paper Account and provider identity without exposing credentials.
- Paper Funding Target and observed starting Account Snapshot.
- Valuation Currency and exact decimal balances.
- Strategy allocation, reserved cash, buying power, positions, and pending orders.
- Every balance-changing Fill, fee, adjustment, and reset event.
- Risk Policy, Execution Profile, Trading Calendar, and market-rule identities.
- Connection health and the age of the latest Account Snapshot.
- Paper Execution Adapter and Capability Snapshot identities.
- Reconciliation state, last successful reconciliation time, and unresolved differences.
- Any difference between provider-reported and locally derived equity.

## What V1 deliberately prevents

- Adding unlike currencies into a meaningless global total.
- A local preference overwriting an external broker balance.
- One Portfolio spending capital held in another Account.
- A Component accessing account credentials or directly changing the ledger.
- A Backtest balance being presented as a Paper Account balance.

## Future Global Portfolio prerequisites

A cross-account or cross-currency Portfolio requires all of the following before it is trustworthy:

- Immutable FX Snapshots and one reporting currency.
- Currency conversion and cash-transfer rules.
- Settlement, borrowing, margin, and collateral semantics.
- Coordinated calendars and decision times.
- Cross-account risk reservations and execution recovery.
- Provider-specific funding and transfer evidence.

Those capabilities are outside V1 and cannot be simulated by a UI-only currency conversion.
