// strategy_validator.rs
// Valida que una estrategia Rhai cumple con estándares mínimos

use std::collections::HashMap;
use regex::Regex;

/// Resultado de validación
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub metrics: StrategyMetrics,
}

/// Métricas de la estrategia
#[derive(Debug, Clone)]
pub struct StrategyMetrics {
    pub functions_found: usize,
    pub config_params: usize,
    pub indicators_used: Vec<String>,
    pub estimated_complexity: String,
}

/// Validador de estrategias
pub struct StrategyValidator;

impl StrategyValidator {
    /// Validar un script Rhai completo
    pub fn validate_strategy(script: &str) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut metrics = StrategyMetrics {
            functions_found: 0,
            config_params: 0,
            indicators_used: Vec::new(),
            estimated_complexity: "Unknown".to_string(),
        };

        // 1. Validaciones críticas
        Self::validate_syntax(script, &mut errors);
        Self::validate_required_functions(script, &mut errors, &mut metrics);
        Self::validate_risk_management(script, &mut errors);
        Self::validate_signal_generation(script, &mut errors);

        // 2. Validaciones recomendadas
        Self::validate_indicators(script, &mut warnings, &mut metrics);
        Self::validate_documentation(script, &mut warnings);
        Self::validate_parameter_ranges(script, &mut warnings);

        // 3. Métricas
        metrics.functions_found = Self::count_functions(script);
        metrics.config_params = Self::count_config_params(script);
        metrics.estimated_complexity = Self::estimate_complexity(&metrics);

        let is_valid = errors.is_empty();

        ValidationResult {
            is_valid,
            errors,
            warnings,
            metrics,
        }
    }

    // ==================== VALIDACIONES CRÍTICAS ====================

    fn validate_syntax(script: &str, errors: &mut Vec<String>) {
        // Verificar que no hay errores sintácticos obvios
        
        // Paréntesis balanceados
        let mut paren_count = 0;
        let mut bracket_count = 0;
        let mut brace_count = 0;

        for ch in script.chars() {
            match ch {
                '(' => paren_count += 1,
                ')' => paren_count -= 1,
                '[' => bracket_count += 1,
                ']' => bracket_count -= 1,
                '{' => brace_count += 1,
                '}' => brace_count -= 1,
                _ => {}
            }
        }

        if paren_count != 0 {
            errors.push("ERROR: Paréntesis desbalanceados".to_string());
        }
        if bracket_count != 0 {
            errors.push("ERROR: Corchetes desbalanceados".to_string());
        }
        if brace_count != 0 {
            errors.push("ERROR: Llaves desbalanceadas".to_string());
        }

        // Verificar strings cerrados
        let single_quote_count = script.chars().filter(|c| *c == '\'').count();
        let double_quote_count = script.chars().filter(|c| *c == '"').count();

        if single_quote_count % 2 != 0 {
            errors.push("ERROR: Comillas simples desbalanceadas".to_string());
        }
        if double_quote_count % 2 != 0 {
            errors.push("ERROR: Comillas dobles desbalanceadas".to_string());
        }
    }

    fn validate_required_functions(
        script: &str,
        errors: &mut Vec<String>,
        metrics: &mut StrategyMetrics,
    ) {
        let required_functions = vec![
            "on_candle",
            "generate_signal",
            "calculate_sma",
            "calculate_atr",
            "calculate_rsi",
        ];

        for func in required_functions {
            if !script.contains(&format!("fn {}", func)) {
                errors.push(format!("ERROR: Función requerida '{}' no encontrada", func));
            }
        }

        // Contar funciones totales
        let func_pattern = Regex::new(r"fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(").unwrap();
        metrics.functions_found = func_pattern.find_iter(script).count();

        if metrics.functions_found < 5 {
            errors.push(format!(
                "ERROR: Muy pocas funciones ({}, mínimo 5)",
                metrics.functions_found
            ));
        }
    }

    fn validate_risk_management(script: &str, errors: &mut Vec<String>) {
        // Verificar que hay configuración de stop loss
        if !script.contains("max_loss_percent") && !script.contains("stop_loss") {
            errors.push("ERROR: No hay configuración de stop loss (max_loss_percent)".to_string());
        }

        // Verificar que hay configuración de take profit
        if !script.contains("take_profit_percent") && !script.contains("take_profit") {
            errors.push("ERROR: No hay configuración de take profit".to_string());
        }

        // Verificar que hay position sizing
        if !script.contains("max_position_size") && !script.contains("position_size") {
            errors.push("ERROR: No hay configuración de tamaño de posición".to_string());
        }

        // Verificar que position_size se calcula
        if !script.contains("calculate_position_size") && !script.contains("position_size =") {
            errors.push("ERROR: No se calcula el tamaño de posición dinámicamente".to_string());
        }
    }

    fn validate_signal_generation(script: &str, errors: &mut Vec<String>) {
        // Verificar que generate_signal retorna valores válidos
        if !script.contains("SIGNAL_BULLISH") && !script.contains("return 1") {
            errors.push(
                "ERROR: No hay señal BULLISH (SIGNAL_BULLISH o return 1)".to_string(),
            );
        }

        if !script.contains("SIGNAL_BEARISH") && !script.contains("return -1") {
            errors.push(
                "ERROR: No hay señal BEARISH (SIGNAL_BEARISH o return -1)".to_string(),
            );
        }

        if !script.contains("SIGNAL_NEUTRAL") && !script.contains("return 0") {
            errors.push(
                "ERROR: No hay señal NEUTRAL (SIGNAL_NEUTRAL o return 0)".to_string(),
            );
        }
    }

    // ==================== VALIDACIONES RECOMENDADAS ====================

    fn validate_indicators(
        script: &str,
        warnings: &mut Vec<String>,
        metrics: &mut StrategyMetrics,
    ) {
        let indicators = vec![
            ("SMA", "calculate_sma"),
            ("RSI", "calculate_rsi"),
            ("ATR", "calculate_atr"),
            ("Momentum", "calculate_momentum"),
            ("MACD", "calculate_macd"),
            ("Bollinger", "calculate_bollinger"),
            ("Volumen", "calculate_sma_volume"),
        ];

        for (name, func) in indicators {
            if script.contains(func) {
                metrics.indicators_used.push(name.to_string());
            }
        }

        if metrics.indicators_used.is_empty() {
            warnings.push(
                "ADVERTENCIA: No se detectan indicadores técnicos (SMA, RSI, etc.)".to_string(),
            );
        }

        if metrics.indicators_used.len() < 2 {
            warnings.push(
                "ADVERTENCIA: Muy pocos indicadores. Se recomienda mínimo 2-3".to_string(),
            );
        }

        if metrics.indicators_used.len() > 5 {
            warnings
                .push("ADVERTENCIA: Muchos indicadores (>5). Considerar simplificar".to_string());
        }
    }

    fn validate_documentation(script: &str, warnings: &mut Vec<String>) {
        // Contar comentarios
        let comments = script.lines().filter(|l| l.trim().starts_with("//")).count();

        if comments < 5 {
            warnings.push(
                "ADVERTENCIA: Muy pocos comentarios. Se recomienda documentar mejor".to_string(),
            );
        }

        // Verificar que hay comentarios de secciones
        if !script.contains("CONFIGURACIÓN") && !script.contains("CONFIG") {
            warnings.push(
                "ADVERTENCIA: Documentar la sección CONFIGURACIÓN".to_string(),
            );
        }

        if !script.contains("INDICADORES") && !script.contains("SIGNALS") {
            warnings.push(
                "ADVERTENCIA: Documentar la sección INDICADORES".to_string(),
            );
        }
    }

    fn validate_parameter_ranges(script: &str, warnings: &mut Vec<String>) {
        // Extraer valores de configuración y validar rangos

        // momentum_threshold
        if let Some(cap) = Regex::new(r"momentum_threshold:\s*([0-9.]+)")
            .unwrap()
            .captures(script)
        {
            if let Ok(value) = cap[1].parse::<f64>() {
                if value < 0.1 || value > 5.0 {
                    warnings.push(format!(
                        "ADVERTENCIA: momentum_threshold {} está fuera del rango recomendado (0.1-5.0)",
                        value
                    ));
                }
            }
        }

        // max_position_size
        if let Some(cap) = Regex::new(r"max_position_size:\s*([0-9.]+)")
            .unwrap()
            .captures(script)
        {
            if let Ok(value) = cap[1].parse::<f64>() {
                if value < 0.05 || value > 1.0 {
                    warnings.push(format!(
                        "ADVERTENCIA: max_position_size {} está fuera del rango recomendado (0.05-1.0)",
                        value
                    ));
                }
            }
        }

        // max_loss_percent
        if let Some(cap) = Regex::new(r"max_loss_percent:\s*([0-9.]+)")
            .unwrap()
            .captures(script)
        {
            if let Ok(value) = cap[1].parse::<f64>() {
                if value < 0.5 || value > 10.0 {
                    warnings.push(format!(
                        "ADVERTENCIA: max_loss_percent {} está fuera del rango recomendado (0.5-10.0)",
                        value
                    ));
                }
            }
        }

        // required_confirmations
        if let Some(cap) = Regex::new(r"required_confirmations:\s*([0-9]+)")
            .unwrap()
            .captures(script)
        {
            if let Ok(value) = cap[1].parse::<usize>() {
                if value < 1 || value > 4 {
                    warnings.push(format!(
                        "ADVERTENCIA: required_confirmations {} debe estar entre 1 y 4",
                        value
                    ));
                }
            }
        }
    }

    // ==================== UTILIDADES ====================

    fn count_functions(script: &str) -> usize {
        Regex::new(r"fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")
            .unwrap()
            .find_iter(script)
            .count()
    }

    fn count_config_params(script: &str) -> usize {
        Regex::new(r"([a-zA-Z_][a-zA-Z0-9_]*):\s*")
            .unwrap()
            .find_iter(script)
            .count()
    }

    fn estimate_complexity(metrics: &StrategyMetrics) -> String {
        let total_score = metrics.functions_found + metrics.indicators_used.len();

        match total_score {
            0..=5 => "Baja (Recomendado para principiantes)".to_string(),
            6..=10 => "Media (Balanceada)".to_string(),
            11..=15 => "Alta (Para traders experimentados)".to_string(),
            _ => "Muy Alta (Asegúrate de validar bien)".to_string(),
        }
    }
}

// ==================== EJEMPLO DE USO ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_strategy() {
        let strategy = r#"
let config = #{
    momentum_threshold: 0.8,
    max_loss_percent: 2.0,
    take_profit_percent: 3.0,
    max_position_size: 0.2
};

fn on_candle(candle_data, capital) {
    // Process candle
}

fn generate_signal(candle_data) {
    return 1;  // BULLISH
}

fn calculate_sma(data, period) {
    0.0
}

fn calculate_atr(highs, lows, closes, period) {
    0.0
}

fn calculate_rsi(closes, period) {
    50.0
}
        "#;

        let result = StrategyValidator::validate_strategy(strategy);
        assert!(result.is_valid, "Strategy should be valid");
    }

    #[test]
    fn test_invalid_strategy_missing_stop_loss() {
        let strategy = r#"
fn on_candle(candle_data, capital) {
    // No stop loss defined!
}
        "#;

        let result = StrategyValidator::validate_strategy(strategy);
        assert!(!result.is_valid, "Should detect missing stop loss");
    }

    #[test]
    fn test_validation_report() {
        let strategy = r#"
let config = #{
    momentum_threshold: 0.8,
    max_position_size: 0.2,
    max_loss_percent: 2.0,
    take_profit_percent: 3.0
};

fn on_candle(candle_data, capital) { }
fn generate_signal(candle_data) { return 1; }
fn calculate_sma(data, period) { 0.0 }
fn calculate_atr(h, l, c, p) { 0.0 }
fn calculate_rsi(c, p) { 50.0 }
        "#;

        let result = StrategyValidator::validate_strategy(strategy);

        println!("\n=== VALIDATION REPORT ===");
        println!("Valid: {}", result.is_valid);
        println!("Functions: {}", result.metrics.functions_found);
        println!("Indicators: {:?}", result.metrics.indicators_used);
        println!("Complexity: {}", result.metrics.estimated_complexity);

        if !result.errors.is_empty() {
            println!("\nErrors:");
            for err in &result.errors {
                println!("  - {}", err);
            }
        }

        if !result.warnings.is_empty() {
            println!("\nWarnings:");
            for warn in &result.warnings {
                println!("  - {}", warn);
            }
        }
    }
}

// ==================== COMANDOS DE USO ====================

/*
PARA VALIDAR UNA ESTRATEGIA:

1. Desde Rust:
   let result = StrategyValidator::validate_strategy(&script);
   if result.is_valid {
       println!("✓ Estrategia válida");
   } else {
       for error in result.errors {
           println!("✗ {}", error);
       }
   }

2. Ver reporte completo:
   println!("{:#?}", result);

3. Mejorar estrategia basado en warnings:
   for warning in result.warnings {
       println!("⚠ {}", warning);
   }
*/
