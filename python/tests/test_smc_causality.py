"""Synthetic regressions: no external market data or network required."""
import importlib.util
from pathlib import Path
import numpy as np
import pandas as pd
import pytest
from aegis_lab.research import smc, smc_models, smc_search, ict_full


def frame(n=720, seed=11):
    rng = np.random.default_rng(seed)
    c = 100 + np.cumsum(rng.normal(0, .15, n))
    o = np.r_[c[0], c[:-1]]
    return pd.DataFrame(dict(t=np.arange(n, dtype=np.int64) * 60, o=o,
                             h=np.maximum(o, c) + .2, l=np.minimum(o, c) - .2,
                             c=c, v=np.ones(n)))


def replay(o, h, l, c, d=1, mode=0, he=None, rr=1., be=0.):
    o, h, l, c = [np.array(a, dtype=float) for a in (o, h, l, c)]
    n = len(o)
    if d < 0:
        o, h, l, c = 200-o, 200-l, 200-h, 200-c
    return smc._sim_details(np.arange(n, dtype=np.int64)*60, h, l, c,
                           np.zeros(n, bool) if he is None else np.array(he, bool),
                           np.ones(n, bool), 0, n, n, d, 100. if d > 0 else 102., 98. if d > 0 else 100., mode, 3, 0., 1.,
                           .1, rr, be, 99999, 1, .01, o, .05, .02)


@pytest.mark.parametrize('d', [1, -1])
def test_ambiguous_stop_first_and_short_mirror(d):
    r = replay([101,100], [101,104], [99.5,97], [100,101], d=d)
    assert r[5] == 0 and r[7] == 1
    assert r[3] == pytest.approx(-2.16)


def test_fill_bar_target_not_backdated():
    r = replay([101,100], [105,101], [99.5,99], [100,100])
    assert r[5] == 4  # high on the fill minute is not known to occur after entry


@pytest.mark.parametrize('d', [1, -1])
def test_gap_stop_uses_open_not_stale_stop(d):
    r = replay([101,95], [101,97], [99.5,94], [100,95], d=d)
    assert r[5] == 0 and r[3] == pytest.approx(-5.06)


def test_hour_close_cannot_cancel_earlier_fill():
    r = replay([101,99], [101,100], [97,98], [97.5,99], he=[True,False])
    assert r[0] == 1 and r[5] == 0 and r[6] == 0


def test_grid_adverse_refills_before_ambiguous_stop():
    r = replay([101,100], [101,110], [99.5,97], [100,100], mode=1)
    assert r[0] == pytest.approx(1.) and r[5] == 0


def test_breakeven_is_not_activated_retroactively():
    r = replay([101,100,100], [101,102.2,100.1], [99.5,99.5,99], [100,101,99.5], rr=3., be=1.)
    assert r[5] == 2 and r[7] == 2


def test_incomplete_and_gapped_htf_bars_unavailable():
    m = frame(119)
    assert len(smc.resample(m, 3600)) == 1
    gapped = frame(120).drop(index=5)
    assert list(smc.resample(gapped, 3600).t) == [3600]


@pytest.mark.parametrize('sec', [3600,14400])
def test_zones_known_only_after_htf_close_and_prefix_mutation(sec):
    m = frame(60*100)
    cutoff = 60*60*60
    before = smc.zones(smc.resample(m[m.t+60 <= cutoff], sec), sec, k=1)
    mutated = m.copy()
    mask = mutated.t >= cutoff
    mutated.loc[mask, ['o','h','l','c']] += 1000
    after = smc.zones(smc.resample(mutated, sec), sec, k=1)
    pd.testing.assert_frame_equal(before.reset_index(drop=True),
                                  after[after.valid <= cutoff].reset_index(drop=True))
    bars = smc.resample(m, sec)
    z = smc.zones(bars, sec, k=1)
    assert len(z) > 0
    assert (z.valid.to_numpy() == bars.t.to_numpy()[z.bar.to_numpy()] + sec).all()


def test_swing_requires_right_hand_confirmation():
    b = pd.DataFrame(dict(t=np.arange(5)*3600, o=[101,101,101,101,106], h=[101,105,102,104,106],
                          l=[99]*5, c=[100,100,100,100,106], atr=[1.]*5))
    assert smc.zones(b.iloc[:2],3600,k=1).query("kind == 'ob'").empty
    z = smc.zones(b,3600,k=1).query("kind == 'ob'")
    assert len(z) == 1 and z.valid.iloc[0] == 5*3600


def injected_lab():
    m = pd.DataFrame(dict(t=[0,60,120,180,240],o=[101,100,100,100,100],
                          h=[101,101,101,104,104], l=[99.5,99,99,99,99], c=[100]*5,v=[1]*5))
    lab = smc.Lab(m=m, slippage=.05, spread=.02)
    lab.z[3600] = pd.DataFrame([dict(kind='ob',dir=1,valid=t,top=100.,bot=98.,atr=1.,
                                   trend=1,pd_ok=1.,size_atr=2.,bar=0) for t in [0,60]])
    return lab


def test_global_one_position_true_exit_and_fees():
    lab = injected_lab()
    cfg = dict(smc.BASE, rr=1.)
    all_trades = lab.run(dict(cfg, portfolio='independent'))
    trades = lab.run(cfg)
    assert len(all_trades) == 2 and len(trades) == 1
    assert trades.exit_t.iloc[0] == 240
    assert trades.fee_total.iloc[0] == pytest.approx(.0002*100 + .0002*102.1)
    assert lab.summary(trades,0,180)['n'] == 0
    assert lab.summary(trades,0,300)['n'] == 1
    assert lab.summary(lab.run(cfg,end=180),0,180)['n'] == 0


def test_empty_lab_and_empty_zones():
    with pytest.raises(ValueError):
        smc.Lab(m=frame(1).iloc[:0])
    lab = smc.Lab(m=frame(10))
    assert lab.run(smc.BASE).empty


def test_model_true_exit_gap_and_compatibility_interface():
    o=np.array([100.,100.,95.]); h=np.array([101.,101.,96.]); l=np.array([99.5,99.5,94.]); c=o.copy()
    args=(o,h,l,c,0,3,1,np.nan,98.,4,1.,1.,.1,1,.7,.1,100,10,.01)
    detail=smc_models._trade_details(*args)
    assert detail[3] == 0 and detail[7] == 2 and detail[2] < -5.
    assert len(smc_models._trade(*args)) == 7


def test_search_true_exit_and_compatibility_interface():
    o=np.array([101.,95.]); h=np.array([101.,97.]); l=np.array([99.5,94.]); c=o.copy()
    args=(np.array([0,60]),o,h,l,c,np.zeros(2,bool),np.ones(2,bool),0,2,1,100.,98.,np.nan,
          0,3,0.,1.,.1,1.,False,1.,0.,1.,0.,1000,1,15,.01)
    detail=smc_search._sim_details(*args)
    assert detail[6] == 0 and detail[8] == 1
    assert len(smc_search._sim(*args)) == 8


def test_ict_fill_bar_gap_stop_is_pessimistic():
    assert ict_full.exit_r(np.array([95.]),np.array([105.]),np.array([94.]),
                           np.array([0]),0,100.,98.,102.,False)[0] == 95.


def benchmark_module():
    path = Path(__file__).resolve().parents[2] / 'scripts/smc_recent_benchmark.py'
    spec = importlib.util.spec_from_file_location('benchmark',path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_frozen_grid_and_selection_never_uses_test():
    mod=benchmark_module()
    configs=mod.candidates()
    assert len(configs)==36
    assert {c['rr'] for c in configs} == {1.,2.,3.}
    assert {c['buf'] for c in configs} == {.1,.25}
    rows=[dict(candidate=i,train_n=20,train_r_net_sum=10-i,validation_n=4,
               validation_r_net_sum=i,test_r_net_sum=10000*(10-i)) for i in range(4)]
    assert mod.select(rows,10,3)[0] == 2
    for row in rows:
        row['test_r_net_sum']=-1e9
    assert mod.select(rows,10,3)[0] == 2
    assert mod.timestamp('2026-09-16T00:00:00Z') == mod.timestamp('2026-09-16')


def test_main_legacy_tuple_interface():
    a = np.array([101.,100.]); h=np.array([101.,101.]); l=np.array([99.5,99.]); c=np.array([100.,100.])
    result = smc._sim(np.array([0,60]),h,l,c,np.zeros(2,bool),np.ones(2,bool),0,2,2,1,
                      100.,98.,0,3,0.,1.,.1,1.,0.,1000,1,.01)
    assert len(result) == 7


def test_market_atr_has_no_future_backfill():
    m = frame(60*20)
    before = smc_models.Market(m.iloc[:60*8], 'binance')
    mutated = m.copy()
    mutated.loc[mutated.t >= 60*60*8, ['o','h','l','c']] += 1000
    after = smc_models.Market(mutated, 'binance')
    np.testing.assert_allclose(before.atr, after.atr[:len(before.atr)], equal_nan=True)
    assert np.isnan(before.atr[:60*5]).all()


def test_trade_prefix_future_mutation_with_settlement_cutoff():
    lab = injected_lab()
    original = lab.run(dict(smc.BASE,rr=1.),end=180)
    m = lab.m.copy()
    m.loc[m.t >= 180,['o','h','l','c']] += 1000
    mutated = smc.Lab(m=m, slippage=.05, spread=.02)
    mutated.z = lab.z
    pd.testing.assert_frame_equal(original,mutated.run(dict(smc.BASE,rr=1.),end=180))


def test_explicit_fvg_is_unavailable_before_third_bar_closes():
    m = frame(180)
    for i,(lo,hi) in enumerate(((99.,100.),(100.,101.),(102.,103.))):
        m.loc[i*60:(i+1)*60-1,['o','c']] = (lo+hi)/2
        m.loc[i*60:(i+1)*60-1,'l'] = lo
        m.loc[i*60:(i+1)*60-1,'h'] = hi
    bars=smc.resample(m,3600)
    bars['atr']=1.
    z=smc.zones(bars,3600,k=1)
    fvg=z[z.kind=='fvg']
    assert len(fvg)==1 and fvg.valid.iloc[0]==10800
    truncated=smc.resample(m.iloc[:-1],3600)
    truncated['atr']=1.
    assert smc.zones(truncated,3600,k=1).query("kind == 'fvg'").empty


def test_fresh_cohort_never_rearms_warmup_zones():
    lab = injected_lab()
    assert lab.run(smc.BASE, start=120).empty


def test_incomplete_prior_bar_cannot_contaminate_later_atr():
    m = frame(60 * 8).drop(index=5)
    changed = m.copy()
    changed.loc[changed.t < 3600, 'h'] += 1000000
    a = smc.resample(m, 3600)
    b = smc.resample(changed, 3600)
    assert np.isfinite(a.atr.iloc[-1])
    pd.testing.assert_frame_equal(a, b)


def test_alternate_limit_fill_gap_uses_actual_adverse_open():
    o = np.array([100.,100.,100.,100.,100.,100.,95.,95.])
    h = np.array([101.,102.,105.,104.,104.,108.,96.,96.])
    l = np.array([99.,99.,100.,99.,98.,100.,94.,94.])
    c = np.array([100.,101.,104.,100.,99.,107.,95.,95.])
    result = smc_models._trade_details(o,h,l,c,4,6,1,np.nan,98.,3,2.,1.,.1,1,.7,.1,100,10,.01)
    assert result[5] is True
    assert result[4] == result[7] == 6
    assert result[2] == pytest.approx(95. - .05 - result[0])


def test_legacy_smc_long_wrapper_contract_remains_available():
    from aegis_lab.research import smc_long
    assert callable(smc_long.smc._sim)
    o = np.array([101.,100.]); h = np.array([101.,104.]); l = np.array([99.5,97.]); c = np.array([100.,101.])
    result = smc_long.smc._sim(np.array([0,60],dtype=np.int64),h,l,c,np.zeros(2,bool),np.ones(2,bool),
                             0,2,2,1,100.,98.,0,3,0.,1.,.1,1.,0.,99999,1,.01)
    assert len(result) == 7


def test_new_cohort_cannot_resurrect_warmup_zones():
    lab = injected_lab()
    lab.z[3600] = lab.z[3600].iloc[:1].copy()  # zone known at t=0
    assert not lab.run(dict(smc.BASE, rr=1.)).empty
    assert lab.run(dict(smc.BASE, rr=1.), start=60).empty


def test_incomplete_prior_bar_cannot_contaminate_atr():
    m = frame(1200)
    gapped = m[m.t != 10 * 60].copy()
    changed = gapped.copy()
    changed.loc[changed.t < 3600, ['h','l']] = [10000., 1.]
    a = smc.resample(gapped, 3600)
    b = smc.resample(changed, 3600)
    assert 0 not in a.t.to_list()
    np.testing.assert_allclose(a.atr.to_numpy(), b.atr.to_numpy(), equal_nan=True)


@pytest.mark.parametrize('bad', [np.nan, np.inf, -np.inf, -0.01])
def test_benchmark_nonfinite_or_negative_costs_rejected(bad):
    mod = benchmark_module()
    with pytest.raises(ValueError):
        mod.validate_costs(0.0002, bad, 0.05, 0.02, 0.01)


def test_benchmark_zero_costs_are_valid_explicit_diagnostic():
    benchmark_module().validate_costs(0.0, 0.0, 0.0, 0.0, 0.01)
