# System audit and Bounce / SMC verification

Updated draft PR45, no main merge or live orders. Local:97 Rust unit+7integration;24 Python unittest;28SMC pytest;14pyramid inline+17extra regressions. CI explicitly installs research dependencies and runs pytest because unittest alone does NOT run function-style SMC tests. Full workspaceCI pending final head, manual desktop use not tested.

## Fixed
Candle integrity and conflicting duplicates; requestedentrymode calibration isolation; empty-history validation; optimizer bounded planning/overflow; desktop invalidsearch rejectbeforedata load. SMC completeHTFbars beforeATR, confirmedswings, no retrofill, pessimistic ambiguousminutestop/gaps and actualexitfees; one globalposition and competingorder cancellation; trueexitcutoff; noOOS tradecount eligibility; freshcohort zones prevent lifecycle resurrection; alternateSMCmodel gapstop corrected. Pyramid supports preceding2aggressive exactMAINhits and1observedrefill120sec, removal/reinsertion cannot preserve MAIN, finite FIFOvolume. All with regressions.

## SMC recent benchmark
BinanceXAUUSDT July1-Aug15train/Aug16-31validation/Sep1-15test; Juneindicatorwarmup. All36 OB/FVG/sweep x1h/4h xRR1/2/3 xbuffer.1/.25 configs recorded; selection uses no test. Selected FVG4h, limit edge,RR2,buffer.1ATR,trend on,PD/killzone off, oneposition,maxhold24h. Test7trades −3.5872R total,−0.5125R/trade,2/7wins. Spread.20 stress−3.62935R; verify exact JSON. Maker2bp/taker5bp/.05slip. Funding omitted; missingbidask/queue assumptions disclosed. This is transparent finitegrid, NOT AMALGAM-SMC integration and NOT profitable deployment.

## Bounce refill
Aug17–19:2signals,1filled0.05oz,net−$0.11311 at1oz plannedper signal. Sep1–3:0refill-gated entries; no profit evidence. Fixed$1m threshold,.10USDstop,5/15/80pyramid,partial50/20/20/10 and provisional distances unchanged. OneMAIN aggregatedlevel continuity cannot identify individualorders. All daily integrityflags reviewed; rawoutcomes included in exported verification bundle.

## Unresolved
Settings machine_key=SHA256(constant+hostname+username), not secret: requires deliberate OSkeychain migration; ciphertextnotrobustcustody. Noautomaticsecretsmigration orcredentialsinspection. Margin/liquidation/funding/impact/ownqueue unknown; immediatecancellations optimistic. ExactpriceFIFO conservative mayunderfill. Price cohortsstatistics are not mark-to-market returns. SMC24hholdcrossesfunding. Censored labels are diagnosticsettledcohorts, not unbiaseddeployablecashseries. Public archives hasheslocalBybit/officialBinance. Shortnewwindows cannotproveedge.
