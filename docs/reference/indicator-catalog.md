# ADAQ Indicator Catalog

Generated from `catalog.json`; do not edit.

160 Indicators / 179 outputs. Lookback is available from the host Catalog for every entry; `unstablePeriod` is the official TA-Lib flag, not a convergence claim.

## `accbands` (ACCBANDS)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `20`; range `2`–`100000`.

### Outputs

- `upper-band` — Double Array.
- `middle-band` — Double Array.
- `lower-band` — Double Array.

## `acos` (ACOS)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ad` (AD)

Group: Volume Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.
- `volume` — Volume; explicit-volume.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `add` (ADD)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `real-1` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `adosc` (ADOSC)

Group: Volume Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.
- `volume` — Volume; explicit-volume.

### Parameters

- `fast-period` — Integer; default `3`; range `2`–`100000`.
- `slow-period` — Integer; default `10`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `adx` (ADX)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `adxr` (ADXR)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `apo` (APO)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `fast-period` — Integer; default `12`; range `2`–`100000`.
- `slow-period` — Integer; default `26`; range `2`–`100000`.
- `ma-type` — MA Type; default `0`; range ``–``.

### Outputs

- `value` — Double Array.

## `aroon` (AROON)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `aroon-down` — Double Array.
- `aroon-up` — Double Array.

## `aroonosc` (AROONOSC)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `asin` (ASIN)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `atan` (ATAN)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `atr` (ATR)

Group: Volatility Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `avgdev` (AVGDEV)

Group: Price Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `avgprice` (AVGPRICE)

Group: Price Transform. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `bbands` (BBANDS)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `5`; range `2`–`100000`.
- `deviations-up` — Double; default `2.000000e+0`; range `-3.000000e+37`–`3.000000e+37`.
- `deviations-down` — Double; default `2.000000e+0`; range `-3.000000e+37`–`3.000000e+37`.
- `ma-type` — MA Type; default `0`; range ``–``.

### Outputs

- `upper-band` — Double Array.
- `middle-band` — Double Array.
- `lower-band` — Double Array.

## `beta` (BETA)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `real-1` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `5`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `bop` (BOP)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `cci` (CCI)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `cdl2crows` (CDL2CROWS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdl3blackcrows` (CDL3BLACKCROWS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdl3inside` (CDL3INSIDE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdl3linestrike` (CDL3LINESTRIKE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdl3outside` (CDL3OUTSIDE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdl3starsinsouth` (CDL3STARSINSOUTH)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdl3whitesoldiers` (CDL3WHITESOLDIERS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlabandonedbaby` (CDLABANDONEDBABY)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `3.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdladvanceblock` (CDLADVANCEBLOCK)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlbelthold` (CDLBELTHOLD)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlbreakaway` (CDLBREAKAWAY)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlclosingmarubozu` (CDLCLOSINGMARUBOZU)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlconcealbabyswall` (CDLCONCEALBABYSWALL)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlcounterattack` (CDLCOUNTERATTACK)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdldarkcloudcover` (CDLDARKCLOUDCOVER)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `5.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdldoji` (CDLDOJI)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdldojistar` (CDLDOJISTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdldragonflydoji` (CDLDRAGONFLYDOJI)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlengulfing` (CDLENGULFING)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdleveningdojistar` (CDLEVENINGDOJISTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `3.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdleveningstar` (CDLEVENINGSTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `3.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdlgapsidesidewhite` (CDLGAPSIDESIDEWHITE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlgravestonedoji` (CDLGRAVESTONEDOJI)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlhammer` (CDLHAMMER)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlhangingman` (CDLHANGINGMAN)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlharami` (CDLHARAMI)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlharamicross` (CDLHARAMICROSS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlhighwave` (CDLHIGHWAVE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlhikkake` (CDLHIKKAKE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlhikkakemod` (CDLHIKKAKEMOD)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlhomingpigeon` (CDLHOMINGPIGEON)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlidentical3crows` (CDLIDENTICAL3CROWS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlinneck` (CDLINNECK)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlinvertedhammer` (CDLINVERTEDHAMMER)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlkicking` (CDLKICKING)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlkickingbylength` (CDLKICKINGBYLENGTH)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlladderbottom` (CDLLADDERBOTTOM)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdllongleggeddoji` (CDLLONGLEGGEDDOJI)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdllongline` (CDLLONGLINE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlmarubozu` (CDLMARUBOZU)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlmatchinglow` (CDLMATCHINGLOW)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlmathold` (CDLMATHOLD)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `5.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdlmorningdojistar` (CDLMORNINGDOJISTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `3.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdlmorningstar` (CDLMORNINGSTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `penetration` — Double; default `3.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Integer Array.

## `cdlonneck` (CDLONNECK)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlpiercing` (CDLPIERCING)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlrickshawman` (CDLRICKSHAWMAN)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlrisefall3methods` (CDLRISEFALL3METHODS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlseparatinglines` (CDLSEPARATINGLINES)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlshootingstar` (CDLSHOOTINGSTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlshortline` (CDLSHORTLINE)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlspinningtop` (CDLSPINNINGTOP)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlstalledpattern` (CDLSTALLEDPATTERN)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlsticksandwich` (CDLSTICKSANDWICH)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdltakuri` (CDLTAKURI)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdltasukigap` (CDLTASUKIGAP)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlthrusting` (CDLTHRUSTING)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdltristar` (CDLTRISTAR)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlunique3river` (CDLUNIQUE3RIVER)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlupsidegap2crows` (CDLUPSIDEGAP2CROWS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `cdlxsidegap3methods` (CDLXSIDEGAP3METHODS)

Group: Pattern Recognition. Official Unstable Period: `false`.

### Inputs

- `open` — Open; fixed-market.
- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `ceil` (CEIL)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `cmo` (CMO)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `correl` (CORREL)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `real-1` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `cos` (COS)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `cosh` (COSH)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `dema` (DEMA)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `div` (DIV)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `real-1` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `dx` (DX)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `ema` (EMA)

Group: Overlap Studies. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `exp` (EXP)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `floor` (FLOOR)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ht-dcperiod` (HT_DCPERIOD)

Group: Cycle Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ht-dcphase` (HT_DCPHASE)

Group: Cycle Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ht-phasor` (HT_PHASOR)

Group: Cycle Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `in-phase` — Double Array.
- `quadrature` — Double Array.

## `ht-sine` (HT_SINE)

Group: Cycle Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `sine` — Double Array.
- `lead-sine` — Double Array.

## `ht-trendline` (HT_TRENDLINE)

Group: Overlap Studies. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ht-trendmode` (HT_TRENDMODE)

Group: Cycle Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Integer Array.

## `imi` (IMI)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `open` — Open; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `kama` (KAMA)

Group: Overlap Studies. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `linearreg` (LINEARREG)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `linearreg-angle` (LINEARREG_ANGLE)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `linearreg-intercept` (LINEARREG_INTERCEPT)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `linearreg-slope` (LINEARREG_SLOPE)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `ln` (LN)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `log10` (LOG10)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ma` (MA)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.
- `ma-type` — MA Type; default `0`; range ``–``.

### Outputs

- `value` — Double Array.

## `macd` (MACD)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `fast-period` — Integer; default `12`; range `2`–`100000`.
- `slow-period` — Integer; default `26`; range `2`–`100000`.
- `signal-period` — Integer; default `9`; range `1`–`100000`.

### Outputs

- `macd` — Double Array.
- `signal` — Double Array.
- `histogram` — Double Array.

## `macdext` (MACDEXT)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `fast-period` — Integer; default `12`; range `2`–`100000`.
- `fast-ma` — MA Type; default `0`; range ``–``.
- `slow-period` — Integer; default `26`; range `2`–`100000`.
- `slow-ma` — MA Type; default `0`; range ``–``.
- `signal-period` — Integer; default `9`; range `1`–`100000`.
- `signal-ma` — MA Type; default `0`; range ``–``.

### Outputs

- `macd` — Double Array.
- `signal` — Double Array.
- `histogram` — Double Array.

## `macdfix` (MACDFIX)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `signal-period` — Integer; default `9`; range `1`–`100000`.

### Outputs

- `macd` — Double Array.
- `signal` — Double Array.
- `histogram` — Double Array.

## `mama` (MAMA)

Group: Overlap Studies. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `fast-limit` — Double; default `5.000000e-1`; range `1.000000e-2`–`9.900000e-1`.
- `slow-limit` — Double; default `5.000000e-2`; range `1.000000e-2`–`9.900000e-1`.

### Outputs

- `mama` — Double Array.
- `fama` — Double Array.

## `max` (MAX)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `maxindex` (MAXINDEX)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `value` — Integer Array.

## `medprice` (MEDPRICE)

Group: Price Transform. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `mfi` (MFI)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.
- `volume` — Volume; explicit-volume.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `midpoint` (MIDPOINT)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `midprice` (MIDPRICE)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `min` (MIN)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `minindex` (MININDEX)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `value` — Integer Array.

## `minmax` (MINMAX)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `min` — Double Array.
- `max` — Double Array.

## `minmaxindex` (MINMAXINDEX)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `min-idx` — Integer Array.
- `max-idx` — Integer Array.

## `minus-di` (MINUS_DI)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `minus-dm` (MINUS_DM)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `mom` (MOM)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `10`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `mult` (MULT)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `real-1` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `natr` (NATR)

Group: Volatility Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `obv` (OBV)

Group: Volume Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `volume` — Volume; explicit-volume.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `plus-di` (PLUS_DI)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `plus-dm` (PLUS_DM)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `ppo` (PPO)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `fast-period` — Integer; default `12`; range `2`–`100000`.
- `slow-period` — Integer; default `26`; range `2`–`100000`.
- `ma-type` — MA Type; default `0`; range ``–``.

### Outputs

- `value` — Double Array.

## `roc` (ROC)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `10`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `rocp` (ROCP)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `10`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `rocr` (ROCR)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `10`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `rocr100` (ROCR100)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `10`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `rsi` (RSI)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `sar` (SAR)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `acceleration-factor` — Double; default `2.000000e-2`; range `0.000000e+0`–`3.000000e+37`.
- `af-maximum` — Double; default `2.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Double Array.

## `sarext` (SAREXT)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.

### Parameters

- `start-value` — Double; default `0.000000e+0`; range `-3.000000e+37`–`3.000000e+37`.
- `offset-on-reverse` — Double; default `0.000000e+0`; range `0.000000e+0`–`3.000000e+37`.
- `af-init-long` — Double; default `2.000000e-2`; range `0.000000e+0`–`3.000000e+37`.
- `af-long` — Double; default `2.000000e-2`; range `0.000000e+0`–`3.000000e+37`.
- `af-max-long` — Double; default `2.000000e-1`; range `0.000000e+0`–`3.000000e+37`.
- `af-init-short` — Double; default `2.000000e-2`; range `0.000000e+0`–`3.000000e+37`.
- `af-short` — Double; default `2.000000e-2`; range `0.000000e+0`–`3.000000e+37`.
- `af-max-short` — Double; default `2.000000e-1`; range `0.000000e+0`–`3.000000e+37`.

### Outputs

- `value` — Double Array.

## `sin` (SIN)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `sinh` (SINH)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `sma` (SMA)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `sqrt` (SQRT)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `stddev` (STDDEV)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `5`; range `2`–`100000`.
- `deviations` — Double; default `1.000000e+0`; range `-3.000000e+37`–`3.000000e+37`.

### Outputs

- `value` — Double Array.

## `stoch` (STOCH)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `fast-k-period` — Integer; default `5`; range `1`–`100000`.
- `slow-k-period` — Integer; default `3`; range `1`–`100000`.
- `slow-k-ma` — MA Type; default `0`; range ``–``.
- `slow-d-period` — Integer; default `3`; range `1`–`100000`.
- `slow-d-ma` — MA Type; default `0`; range ``–``.

### Outputs

- `slow-k` — Double Array.
- `slow-d` — Double Array.

## `stochf` (STOCHF)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `fast-k-period` — Integer; default `5`; range `1`–`100000`.
- `fast-d-period` — Integer; default `3`; range `1`–`100000`.
- `fast-d-ma` — MA Type; default `0`; range ``–``.

### Outputs

- `fast-k` — Double Array.
- `fast-d` — Double Array.

## `stochrsi` (STOCHRSI)

Group: Momentum Indicators. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.
- `fast-k-period` — Integer; default `5`; range `1`–`100000`.
- `fast-d-period` — Integer; default `3`; range `1`–`100000`.
- `fast-d-ma` — MA Type; default `0`; range ``–``.

### Outputs

- `fast-k` — Double Array.
- `fast-d` — Double Array.

## `sub` (SUB)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.
- `real-1` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `sum` (SUM)

Group: Math Operators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `t3` (T3)

Group: Overlap Studies. Official Unstable Period: `true`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `5`; range `1`–`100000`.
- `volume-factor` — Double; default `7.000000e-1`; range `0.000000e+0`–`1.000000e+0`.

### Outputs

- `value` — Double Array.

## `tan` (TAN)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `tanh` (TANH)

Group: Math Transform. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `tema` (TEMA)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `trange` (TRANGE)

Group: Volatility Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `trima` (TRIMA)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `trix` (TRIX)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `tsf` (TSF)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `typprice` (TYPPRICE)

Group: Price Transform. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `ultosc` (ULTOSC)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `first-period` — Integer; default `7`; range `1`–`100000`.
- `second-period` — Integer; default `14`; range `1`–`100000`.
- `third-period` — Integer; default `28`; range `1`–`100000`.

### Outputs

- `value` — Double Array.

## `var` (VAR)

Group: Statistic Functions. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `5`; range `1`–`100000`.
- `deviations` — Double; default `1.000000e+0`; range `-3.000000e+37`–`3.000000e+37`.

### Outputs

- `value` — Double Array.

## `wclprice` (WCLPRICE)

Group: Price Transform. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- None.

### Outputs

- `value` — Double Array.

## `willr` (WILLR)

Group: Momentum Indicators. Official Unstable Period: `false`.

### Inputs

- `high` — High; fixed-market.
- `low` — Low; fixed-market.
- `close` — Close; fixed-market.

### Parameters

- `time-period` — Integer; default `14`; range `2`–`100000`.

### Outputs

- `value` — Double Array.

## `wma` (WMA)

Group: Overlap Studies. Official Unstable Period: `false`.

### Inputs

- `real-0` — Double Array; generic-ohcl-real.

### Parameters

- `time-period` — Integer; default `30`; range `1`–`100000`.

### Outputs

- `value` — Double Array.
