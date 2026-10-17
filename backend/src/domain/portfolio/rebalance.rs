use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::portfolio::models::Holding;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetAllocationDrift {
    pub symbol: String,
    pub current_value_minor: i64,
    pub current_weight_pct: f64,
    pub target_weight_pct: f64,
    pub drift_pct: f64,
    pub rebalance_action: RebalanceAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RebalanceAction {
    Buy { amount_minor: i64 },
    Sell { amount_minor: i64 },
    InBand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioDriftReport {
    pub total_value_minor: i64,
    pub allocations: Vec<AssetAllocationDrift>,
    pub max_drift_pct: f64,
    pub needs_rebalance: bool,
}

pub fn evaluate_portfolio_drift(
    holdings: &[Holding],
    targets: &BTreeMap<String, f64>,
    threshold_pct: f64,
) -> PortfolioDriftReport {
    let total_value_minor: i64 = holdings
        .iter()
        .map(|h| h.market_value_minor().unwrap_or(0))
        .sum();

    let mut current_values: BTreeMap<String, i64> = BTreeMap::new();
    let mut symbol_order: Vec<String> = Vec::new();

    for h in holdings {
        let entry = current_values.entry(h.symbol.clone()).or_insert_with(|| {
            symbol_order.push(h.symbol.clone());
            0
        });
        *entry += h.market_value_minor().unwrap_or(0);
    }

    for target_symbol in targets.keys() {
        if !current_values.contains_key(target_symbol) {
            symbol_order.push(target_symbol.clone());
            current_values.insert(target_symbol.clone(), 0);
        }
    }

    let mut allocations = Vec::with_capacity(symbol_order.len());
    let mut max_drift_pct = 0.0_f64;
    let mut needs_rebalance = false;

    for symbol in symbol_order {
        let val = *current_values.get(&symbol).unwrap_or(&0);
        let target_weight_pct = targets.get(&symbol).copied().unwrap_or(0.0);
        let current_weight_pct = if total_value_minor > 0 {
            (val as f64 / total_value_minor as f64) * 100.0
        } else {
            0.0
        };
        let drift_pct = current_weight_pct - target_weight_pct;
        let abs_drift = drift_pct.abs();
        if abs_drift > max_drift_pct {
            max_drift_pct = abs_drift;
        }

        let rebalance_action = if abs_drift > threshold_pct {
            needs_rebalance = true;
            if drift_pct > 0.0 {
                let amount = ((drift_pct / 100.0) * total_value_minor as f64).round() as i64;
                RebalanceAction::Sell {
                    amount_minor: amount,
                }
            } else {
                let amount = (((-drift_pct) / 100.0) * total_value_minor as f64).round() as i64;
                RebalanceAction::Buy {
                    amount_minor: amount,
                }
            }
        } else {
            RebalanceAction::InBand
        };

        allocations.push(AssetAllocationDrift {
            symbol,
            current_value_minor: val,
            current_weight_pct,
            target_weight_pct,
            drift_pct,
            rebalance_action,
        });
    }

    PortfolioDriftReport {
        total_value_minor,
        allocations,
        max_drift_pct,
        needs_rebalance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::portfolio::models::QUANTITY_EXPONENT;

    fn sample_holding(symbol: &str, qty_units: i64, price_cents: i64) -> Holding {
        Holding {
            id: 1,
            owner_id: AccountId::from_seed("user"),
            symbol: symbol.to_string(),
            quantity_minor: qty_units * 10i64.pow(QUANTITY_EXPONENT),
            avg_cost_minor: price_cents,
            currency: "USD".to_string(),
            currency_exponent: 2,
            current_price_minor: Some(price_cents),
            price_updated_at: None,
            note: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn test_single_holding_100_pct_target() {
        let holdings = vec![sample_holding("AAPL", 10, 10_000)]; // $1,000.00
        let mut targets = BTreeMap::new();
        targets.insert("AAPL".to_string(), 100.0);

        let report = evaluate_portfolio_drift(&holdings, &targets, 5.0);
        assert_eq!(report.total_value_minor, 100_000);
        assert_eq!(report.allocations.len(), 1);
        let alloc = &report.allocations[0];
        assert_eq!(alloc.symbol, "AAPL");
        assert_eq!(alloc.current_value_minor, 100_000);
        assert!((alloc.current_weight_pct - 100.0).abs() < 1e-4);
        assert!((alloc.drift_pct - 0.0).abs() < 1e-4);
        assert_eq!(alloc.rebalance_action, RebalanceAction::InBand);
        assert!((report.max_drift_pct - 0.0).abs() < 1e-4);
        assert!(!report.needs_rebalance);
    }

    #[test]
    fn test_balanced_50_50() {
        let holdings = vec![
            sample_holding("AAPL", 5, 10_000), // $500.00
            sample_holding("MSFT", 5, 10_000), // $500.00
        ];
        let mut targets = BTreeMap::new();
        targets.insert("AAPL".to_string(), 50.0);
        targets.insert("MSFT".to_string(), 50.0);

        let report = evaluate_portfolio_drift(&holdings, &targets, 5.0);
        assert_eq!(report.total_value_minor, 100_000);
        assert_eq!(report.allocations.len(), 2);
        for alloc in &report.allocations {
            assert!((alloc.current_weight_pct - 50.0).abs() < 1e-4);
            assert!((alloc.drift_pct - 0.0).abs() < 1e-4);
            assert_eq!(alloc.rebalance_action, RebalanceAction::InBand);
        }
        assert!((report.max_drift_pct - 0.0).abs() < 1e-4);
        assert!(!report.needs_rebalance);
    }

    #[test]
    fn test_drift_exceeding_threshold_producing_buy_sell() {
        // Total value: 700 + 300 = $1,000.00 (100,000 minor)
        // AAPL is 70%, target is 50%, drift is +20% -> Sell $200 (20,000 minor)
        // MSFT is 30%, target is 50%, drift is -20% -> Buy $200 (20,000 minor)
        let holdings = vec![
            sample_holding("AAPL", 7, 10_000),
            sample_holding("MSFT", 3, 10_000),
        ];
        let mut targets = BTreeMap::new();
        targets.insert("AAPL".to_string(), 50.0);
        targets.insert("MSFT".to_string(), 50.0);

        let report = evaluate_portfolio_drift(&holdings, &targets, 5.0);
        assert_eq!(report.total_value_minor, 100_000);
        assert_eq!(report.allocations.len(), 2);

        let aapl = report.allocations.iter().find(|a| a.symbol == "AAPL").unwrap();
        assert!((aapl.current_weight_pct - 70.0).abs() < 1e-4);
        assert!((aapl.drift_pct - 20.0).abs() < 1e-4);
        assert_eq!(aapl.rebalance_action, RebalanceAction::Sell { amount_minor: 20_000 });

        let msft = report.allocations.iter().find(|a| a.symbol == "MSFT").unwrap();
        assert!((msft.current_weight_pct - 30.0).abs() < 1e-4);
        assert!((msft.drift_pct - (-20.0)).abs() < 1e-4);
        assert_eq!(msft.rebalance_action, RebalanceAction::Buy { amount_minor: 20_000 });

        assert!((report.max_drift_pct - 20.0).abs() < 1e-4);
        assert!(report.needs_rebalance);
    }

    #[test]
    fn test_zero_portfolio_value() {
        let holdings: Vec<Holding> = vec![];
        let mut targets = BTreeMap::new();
        targets.insert("AAPL".to_string(), 100.0);

        let report = evaluate_portfolio_drift(&holdings, &targets, 5.0);
        assert_eq!(report.total_value_minor, 0);
        assert_eq!(report.allocations.len(), 1);
        let alloc = &report.allocations[0];
        assert_eq!(alloc.symbol, "AAPL");
        assert_eq!(alloc.current_value_minor, 0);
        assert_eq!(alloc.current_weight_pct, 0.0);
        assert_eq!(alloc.drift_pct, -100.0);
        assert_eq!(alloc.rebalance_action, RebalanceAction::Buy { amount_minor: 0 });
        assert_eq!(report.max_drift_pct, 100.0);
        assert!(report.needs_rebalance);
    }

    #[test]
    fn test_unheld_target_asset() {
        let holdings = vec![sample_holding("AAPL", 10, 10_000)]; // $1,000.00
        let mut targets = BTreeMap::new();
        targets.insert("AAPL".to_string(), 60.0);
        targets.insert("GOOG".to_string(), 40.0);

        let report = evaluate_portfolio_drift(&holdings, &targets, 5.0);
        assert_eq!(report.total_value_minor, 100_000);
        assert_eq!(report.allocations.len(), 2);

        let goog = report.allocations.iter().find(|a| a.symbol == "GOOG").unwrap();
        assert_eq!(goog.current_value_minor, 0);
        assert_eq!(goog.current_weight_pct, 0.0);
        assert_eq!(goog.target_weight_pct, 40.0);
        assert_eq!(goog.drift_pct, -40.0);
        assert_eq!(goog.rebalance_action, RebalanceAction::Buy { amount_minor: 40_000 });

        let aapl = report.allocations.iter().find(|a| a.symbol == "AAPL").unwrap();
        assert_eq!(aapl.current_weight_pct, 100.0);
        assert_eq!(aapl.target_weight_pct, 60.0);
        assert_eq!(aapl.drift_pct, 40.0);
        assert_eq!(aapl.rebalance_action, RebalanceAction::Sell { amount_minor: 40_000 });
    }
}
