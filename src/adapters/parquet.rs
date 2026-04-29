use crate::adapters::provider::{SourceDataFormat, SourceFieldType, SourceSchema};
use crate::domain::errors::BacktestError;
use crate::domain::types::MarketDataSet;

pub fn validate_schema_for_dataset(
    schema: &SourceSchema,
    dataset: &MarketDataSet,
) -> Result<(), BacktestError> {
    if schema.format != SourceDataFormat::Parquet {
        return Ok(());
    }

    let required_fields = match dataset {
        MarketDataSet::Candles(_) => candle_required_fields(),
        MarketDataSet::Events(_) => event_required_fields(),
    };

    for (field_name, expected_type) in required_fields {
        let Some(field) = schema.fields.iter().find(|field| field.name == *field_name) else {
            return Err(BacktestError::InvalidData(format!(
                "parquet schema is missing required field '{field_name}'"
            )));
        };
        if field.data_type != *expected_type {
            return Err(BacktestError::InvalidData(format!(
                "parquet field '{}' has type {:?} but {:?} was required",
                field_name, field.data_type, expected_type
            )));
        }
    }

    Ok(())
}

pub fn candle_required_fields() -> &'static [(&'static str, SourceFieldType)] {
    &[
        ("timestamp", SourceFieldType::Timestamp),
        ("open", SourceFieldType::Float64),
        ("high", SourceFieldType::Float64),
        ("low", SourceFieldType::Float64),
        ("close", SourceFieldType::Float64),
        ("volume", SourceFieldType::Float64),
    ]
}

pub fn event_required_fields() -> &'static [(&'static str, SourceFieldType)] {
    &[
        ("timestamp", SourceFieldType::Timestamp),
        ("sequence", SourceFieldType::UInt64),
        ("kind", SourceFieldType::Utf8),
    ]
}

#[cfg(test)]
mod tests {
    use super::validate_schema_for_dataset;
    use crate::adapters::provider::{SourceDataFormat, SourceField, SourceFieldType, SourceSchema};
    use crate::domain::types::MarketDataSet;

    #[test]
    fn parquet_schema_validation_requires_event_fields() {
        let schema = SourceSchema {
            format: SourceDataFormat::Parquet,
            fields: vec![
                SourceField {
                    name: "timestamp".to_string(),
                    data_type: SourceFieldType::Timestamp,
                    nullable: false,
                },
                SourceField {
                    name: "sequence".to_string(),
                    data_type: SourceFieldType::UInt64,
                    nullable: false,
                },
            ],
        };
        let error =
            validate_schema_for_dataset(&schema, &MarketDataSet::Events(vec![])).unwrap_err();
        assert!(matches!(
            error,
            crate::domain::errors::BacktestError::InvalidData(_)
        ));
    }
}
