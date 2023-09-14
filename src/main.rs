use std::{
    io,
    str::FromStr,
    string::ParseError,
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use rusty_money::{
    iso::{self, Currency},
    Money,
};
use thiserror::Error;

#[derive(Parser)]
pub struct TaxLotOpts {
    #[clap(subcommand)]
    selection_algo: SelectionAlgorithm,
}

#[derive(Debug, Error)]
pub enum TaxLotError {
    #[error("Malformed input. Failed to parse {field}")]
    ParseError { field: String },
}

#[derive(Debug, Subcommand)]
pub enum SelectionAlgorithm {
    #[clap(name = "fifo")]
    Fifo,

    #[clap(name = "hifo")]
    Hifo,
}

pub enum LotType {
    Buy,
    Sell,
}

impl FromStr for LotType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "buy" => Ok(LotType::Buy),
            "sell" => Ok(LotType::Sell),
            _ => Err("Malformatted input: could not parse lot type".to_string()),
        }
    }
}

struct LotOperation<'a> {
    date: NaiveDate,
    lot_type: LotType,
    price: Money<'a, Currency>,
    quantity: f64,
}

impl<'a> FromStr for LotOperation<'a> {
    type Err = TaxLotError;

    fn from_str(s: &str) -> Result<Self, TaxLotError> {
        let parts: Vec<&str> = s.split(',').collect();
        // TODO: Error checking here

        let date = NaiveDate::parse_from_str(
            parts.get(0).expect("Malformatted input: no date"),
            "%Y-%m-%d",
        )
        .expect("TODO: FIX THIS");
        let lot_type = LotType::from_str(parts.get(1).expect("Malformatted input: no lot type"))
            .expect("TODO: FIX THIS");
        let price = Money::from_str(
            parts.get(2).expect("Malformatted input: no price"),
            iso::USD,
        )
        .expect("TODO: FIX THIS");
        let quantity = parts
            .get(3)
            .expect("Malformatted input: no quantity")
            .parse::<f64>()
            .expect("TODO: FIX THIS");

        Ok(LotOperation {
            date,
            lot_type,
            price,
            quantity,
        })
    }
}

impl<'a> LotOperation<'a> {
    fn create_new_lot(self, id_generator: &'a AtomicU64) -> Lot {
        Lot {
            id: id_generator.fetch_add(1, Ordering::SeqCst),
            date: self.date,
            price: self.price,
            quantity: self.quantity,
        }
    }
}

struct Lot<'a> {
    id: u64,
    date: NaiveDate,
    price: Money<'a, Currency>,
    quantity: f64,
}

fn main() -> Result<(), TaxLotError> {
    let TaxLotOpts { selection_algo } = TaxLotOpts::parse();

    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let lot = LotOperation::from_str(input.as_str())?;
        }
        Err(error) => {}
    }

    Ok(())
}
