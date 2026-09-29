//! DATA strategy from the command line: backtest the Power of 3 rule on a 1m CSV
//! (`time,open,high,low,close[,volume]`, UTC seconds).
//!
//!   data backtest mins.csv [params.json]
use aegis_core::{bounce, data};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(2).ok_or("usage: data backtest mins.csv [params.json]")?;
    let mins = bounce::load_minutes(std::path::Path::new(path))?;
    let p: data::Params = match args.get(3) {
        Some(f) => {
            serde_json::from_str(&std::fs::read_to_string(f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
        }
        None => data::Params::default(),
    };
    let r = data::backtest(&mins, &p);
    let s = &r.stats;
    println!(
        "{} trades, gross {:+.3} R, net {:+.3} R (t {:.2}), win {:.1}%, {:+.2} $/oz, max dd {:.1} R",
        s.trades,
        s.avg_r_gross,
        s.avg_r,
        s.t_stat,
        s.win_rate * 100.0,
        s.avg_usd,
        s.max_dd_r
    );
    for (y, s) in &r.by_year {
        println!(
            "  {y}: {} trades, gross {:+.3} R, net {:+.3} R",
            s.trades, s.avg_r_gross, s.avg_r
        );
    }
    for pk in r.picks.iter().rev().take(3) {
        println!(
            "  auto {}: {}-{} (train {:+.3} R on {})",
            pk.month, pk.entry_min, pk.exit_min, pk.train_r, pk.train_trades
        );
    }
    Ok(())
}
