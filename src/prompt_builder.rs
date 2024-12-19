use std::fmt::Write;

// pub fn build_prompt(eth_data: &[[f64; 6]], btc_data: &[[f64; 6]], sol_data: &[[f64; 6]]) -> String {
//     // Helper function to format a slice of candles as JSON arrays.
//     // This will avoid unnecessary cloning by writing directly to a String via `write!`.
//     fn format_candles(data: &[[f64; 6]]) -> String {
//         let mut s = String::from("[");
//         data.iter().enumerate().for_each(|(i, c)| {
//             if i > 0 {
//                 s.push(',');
//             }
//             // c: [time, open, high, low, close, vol]
//             // We'll present them as arrays for simplicity.
//             let _ = write!(
//                 s,
//                 "[{:.2},{:.2},{:.2},{:.2},{:.2},{:.6}]",
//                 c[0], c[1], c[2], c[3], c[4], c[5]
//             );
//         });
//         s.push(']');
//         s
//     }

//     let eth_json = format_candles(eth_data);
//     let btc_json = format_candles(btc_data);
//     let sol_json = format_candles(sol_data);

//     // Instructions to the model:
//     // - The model receives ETH, BTC, and SOL candle data.
//     // - The model is asked to predict an action for ETH/USD only, assuming BTC might lead and SOL gives more context.
//     // - The model output must be a JSON with "action" and "rationale".
//     // - The rationale should be concise.
//     let mut prompt = String::new();
//     prompt.push_str("You are a trading analysis tool. You have access to recent market data for ETH/USD, BTC/USD, and SOL/USD.\n");
//     prompt.push_str("Your goal is to decide whether to go 'long', 'short', or do 'none' on ETH/USD based on the provided data.\n\n");

//     prompt.push_str("Constraints:\n");
//     prompt.push_str("- You must return a JSON object with two fields:\n");
//     prompt.push_str("  \"action\": one of \"long\", \"short\", or \"none\"\n");
//     prompt.push_str("  \"rationale\": a concise explanation of why this action was chosen\n");
//     prompt.push_str("- Assume BTC generally leads the market and SOL data adds context.\n");
//     prompt.push_str("- Base your decision on the ETH/USD price trends, volatility, and relationship to BTC & SOL.\n\n");

//     prompt.push_str(
//         "Data provided (hourly candles, format: [timestamp, open, high, low, close, volume]):\n",
//     );
//     prompt.push_str("ETH: ");
//     prompt.push_str(&eth_json);
//     prompt.push('\n');
//     prompt.push_str("BTC: ");
//     prompt.push_str(&btc_json);
//     prompt.push('\n');
//     prompt.push_str("SOL: ");
//     prompt.push_str(&sol_json);
//     prompt.push('\n');

//     prompt.push_str("\nInstructions:\n");
//     prompt
//         .push_str("Analyze the data and decide the next probable profitable action for ETH/USD.\n");
//     prompt.push_str("Return the response as a JSON object:\n");
//     prompt.push_str("{\n");
//     prompt.push_str("  \"action\": \"long\" | \"short\" | \"none\",\n");
//     prompt.push_str("  \"rationale\": \"...\"\n");
//     prompt.push_str("}\n");

//     prompt
// }

pub fn build_data_section(
    eth_data: &[[f64; 6]],
    btc_data: &[[f64; 6]],
    sol_data: &[[f64; 6]],
) -> String {
    fn format_candles(data: &[[f64; 6]]) -> String {
        let mut s = String::from("[");
        data.iter().enumerate().for_each(|(i, c)| {
            if i > 0 {
                s.push(',');
            }
            // c: [time, open, high, low, close, vol]
            let _ = write!(
                s,
                "[{:.2},{:.2},{:.2},{:.2},{:.2},{:.6}]",
                c[0], c[1], c[2], c[3], c[4], c[5]
            );
        });
        s.push(']');
        s
    }

    let eth_json = format_candles(eth_data);
    let btc_json = format_candles(btc_data);
    let sol_json = format_candles(sol_data);

    // Now we only return the data portion:
    let mut data_section = String::new();
    data_section.push_str(
        "Data provided (hourly candles, format: [timestamp, open, high, low, close, volume]):\n",
    );
    data_section.push_str("ETH: ");
    data_section.push_str(&eth_json);
    data_section.push('\n');
    data_section.push_str("BTC: ");
    data_section.push_str(&btc_json);
    data_section.push('\n');
    data_section.push_str("SOL: ");
    data_section.push_str(&sol_json);
    data_section.push('\n');

    data_section
}

#[cfg(test)]
mod tests {
    use super::build_data_section;

    #[test]
    fn test_build_prompt() {
        tracing_subscriber::fmt::init();
        let eth_data = [
            [
                1732849200.0,
                3591.36,
                3603.0,
                3599.99,
                3594.88,
                415.86094626,
            ],
            [
                1732845600.0,
                3564.44,
                3600.0,
                3565.45,
                3599.99,
                4979.85077974,
            ],
        ];
        let btc_data = [
            [1732849200.0, 50000.0, 50100.0, 49950.0, 50050.0, 2000.0],
            [1732845600.0, 50100.0, 50200.0, 50000.0, 50080.0, 1900.0],
        ];
        let sol_data = [
            [1732849200.0, 150.0, 152.0, 149.5, 151.0, 10000.0],
            [1732845600.0, 151.0, 153.0, 150.0, 152.0, 8000.0],
        ];

        let prompt = build_data_section(&eth_data, &btc_data, &sol_data);
        tracing::info!(%prompt);
        assert!(prompt.contains("\"action\":"));
        assert!(prompt.contains("\"rationale\":"));
        assert!(prompt.contains("ETH: [[1732849200.00,3591.36,3603.00,3599.99,3594.88,415.860946"));
        assert!(
            prompt.contains("BTC: [[1732849200.00,50000.00,50100.00,49950.00,50050.00,2000.000000")
        );
        assert!(prompt.contains("SOL: [[1732849200.00,150.00,152.00,149.50,151.00,10000.000000"));
    }
}
