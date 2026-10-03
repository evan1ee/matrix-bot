//! Heuristic offline parser extracting key financial fields from raw receipt OCR text.
//!
//! Synchronous, pure domain function operating over strings and decimals.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedReceipt {
    pub merchant: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub date: Option<String>,
    pub category_suggestion: Option<String>,
}

/// Heuristic offline parser extracting key financial fields from raw receipt OCR text.
pub fn extract_receipt_fields(raw_text: &str) -> ExtractedReceipt {
    let lines: Vec<&str> = raw_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        return ExtractedReceipt {
            merchant: None,
            amount: None,
            currency: None,
            date: None,
            category_suggestion: None,
        };
    }

    // 1. Merchant: First clean non-technical line
    let mut merchant: Option<String> = None;
    for line in &lines {
        let lower = line.to_lowercase();
        if lower.contains("welcome")
            || lower.contains("tel:")
            || lower.contains("phone")
            || lower.contains("tax")
            || lower.contains("receipt")
            || lower.contains("invoice")
            || lower.contains("cashier")
            || lower.contains("order #")
            || lower
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == ':')
        {
            continue;
        }
        merchant = Some(line.to_string());
        break;
    }

    // 2. Currency: Search for symbols or ISO codes
    let mut currency: Option<String> = None;
    let text_lower = raw_text.to_lowercase();
    if text_lower.contains("usd") || raw_text.contains('$') {
        currency = Some("USD".to_string());
    } else if text_lower.contains("eur") || raw_text.contains('€') {
        currency = Some("EUR".to_string());
    } else if text_lower.contains("cny")
        || text_lower.contains("rmb")
        || raw_text.contains('¥')
        || text_lower.contains("元")
    {
        currency = Some("CNY".to_string());
    } else if text_lower.contains("gbp") || raw_text.contains('£') {
        currency = Some("GBP".to_string());
    } else if text_lower.contains("jpy") {
        currency = Some("JPY".to_string());
    }

    // 3. Amount: Look for "Total", "Grand Total", "合计", "实收" lines in reverse order, else max valid decimal
    let mut amount: Option<String> = None;
    let mut total_candidate: Option<f64> = None;

    for line in lines.iter().rev() {
        let l_lower = line.to_lowercase();
        let is_subtotal = l_lower.contains("subtotal")
            || l_lower.contains("sub total")
            || l_lower.contains("小计");
        let is_total_line = !is_subtotal
            && (l_lower.contains("total")
                || l_lower.contains("合计")
                || l_lower.contains("总计")
                || l_lower.contains("实收")
                || l_lower.contains("应付")
                || l_lower.contains("amount due"));

        if is_total_line {
            if let Some(num) = find_decimal_in_line(line) {
                total_candidate = Some(num);
                break;
            }
        }
    }

    if total_candidate.is_none() {
        for line in &lines {
            let l_lower = line.to_lowercase();
            if l_lower.contains("date") || l_lower.contains("time") || l_lower.contains("时间") {
                continue;
            }
            if let Some(num) = find_decimal_in_line(line) {
                if total_candidate.is_none() || num > total_candidate.unwrap_or(0.0) {
                    total_candidate = Some(num);
                }
            }
        }
    }

    if let Some(val) = total_candidate {
        amount = Some(format!("{:.2}", val));
    }

    // 4. Date: Look for YYYY-MM-DD or YYYY/MM/DD
    let mut date: Option<String> = None;
    for line in &lines {
        if let Some(found_date) = find_date_in_line(line) {
            date = Some(found_date);
            break;
        }
    }

    // 5. Category Suggestion
    let mut category_suggestion: Option<String> = None;
    if text_lower.contains("coffee")
        || text_lower.contains("cafe")
        || text_lower.contains("restaurant")
        || text_lower.contains("tea")
        || text_lower.contains("starbucks")
        || text_lower.contains("burger")
        || text_lower.contains("pizza")
        || text_lower.contains("餐")
        || text_lower.contains("饮")
        || text_lower.contains("酒")
    {
        category_suggestion = Some("Food & Dining".to_string());
    } else if text_lower.contains("market")
        || text_lower.contains("grocery")
        || text_lower.contains("supermarket")
        || text_lower.contains("mart")
        || text_lower.contains("costco")
        || text_lower.contains("walmart")
        || text_lower.contains("盒马")
        || text_lower.contains("生鲜")
        || text_lower.contains("超市")
        || text_lower.contains("便利店")
    {
        category_suggestion = Some("Groceries".to_string());
    } else if text_lower.contains("metro")
        || text_lower.contains("subway")
        || text_lower.contains("taxi")
        || text_lower.contains("uber")
        || text_lower.contains("lyft")
        || text_lower.contains("flight")
        || text_lower.contains("gas")
        || text_lower.contains("fuel")
        || text_lower.contains("打车")
        || text_lower.contains("交通")
        || text_lower.contains("加油")
    {
        category_suggestion = Some("Transportation".to_string());
    }

    ExtractedReceipt {
        merchant,
        amount,
        currency,
        date,
        category_suggestion,
    }
}

fn find_decimal_in_line(line: &str) -> Option<f64> {
    let clean: String = line
        .chars()
        .map(|c| {
            if c.is_ascii_digit() || c == '.' || c == ',' {
                c
            } else {
                ' '
            }
        })
        .collect();

    let mut last_decimal = None;
    for token in clean.split_whitespace() {
        let norm = token.replace(',', ".");
        if norm.contains('.') {
            if let Ok(val) = norm.parse::<f64>() {
                if (0.01..=1_000_000.0).contains(&val) {
                    last_decimal = Some(val);
                }
            }
        }
    }
    last_decimal
}

fn find_date_in_line(line: &str) -> Option<String> {
    for part in line.split_whitespace() {
        let p = part.trim_matches(|c: char| !c.is_ascii_digit());
        let normalized = p.replace(['/', '.'], "-");
        let segments: Vec<&str> = normalized.split('-').collect();
        if segments.len() == 3 {
            // YYYY-MM-DD
            if segments[0].len() == 4 && segments[1].len() <= 2 && segments[2].len() <= 2 {
                if let (Ok(y), Ok(m), Ok(d)) = (
                    segments[0].parse::<u32>(),
                    segments[1].parse::<u32>(),
                    segments[2].parse::<u32>(),
                ) {
                    if (2000..=2099).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d)
                    {
                        return Some(format!("{:04}-{:02}-{:02}", y, m, d));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_receipt_fields() {
        let sample_receipt = r#"
            STARBUCKS COFFEE #1042
            123 Main St, Seattle WA
            Date: 2026-09-17 10:15 AM
            1x Caramel Macchiato    $5.75
            1x Blueberry Muffin     $3.50
            Subtotal:               $9.25
            Tax (8%):               $0.74
            TOTAL AMOUNT:           $9.99
            Thank you for visiting!
        "#;

        let res = extract_receipt_fields(sample_receipt);
        assert_eq!(res.merchant.as_deref(), Some("STARBUCKS COFFEE #1042"));
        assert_eq!(res.currency.as_deref(), Some("USD"));
        assert_eq!(res.amount.as_deref(), Some("9.99"));
        assert_eq!(res.date.as_deref(), Some("2026-09-17"));
        assert_eq!(res.category_suggestion.as_deref(), Some("Food & Dining"));
    }

    #[test]
    fn test_extract_chinese_receipt() {
        let sample_receipt = r#"
            盒马鲜生 (滨江店)
            订单号: 20260918001
            交易时间: 2026-08-25 18:30
            有机鲜牛奶 950ml        ¥22.50
            特选香蕉 1kg            ¥15.00
            合计:                   ¥37.50
            微信支付实收:           ¥37.50
        "#;

        let res = extract_receipt_fields(sample_receipt);
        assert_eq!(res.merchant.as_deref(), Some("盒马鲜生 (滨江店)"));
        assert_eq!(res.currency.as_deref(), Some("CNY"));
        assert_eq!(res.amount.as_deref(), Some("37.50"));
        assert_eq!(res.date.as_deref(), Some("2026-08-25"));
        assert_eq!(res.category_suggestion.as_deref(), Some("Groceries"));
    }
}
