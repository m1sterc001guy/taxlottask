use std::{
    collections::VecDeque,
    fmt::Display,
    io,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering}, process,
};

use chrono::{NaiveDate, ParseError};
use clap::{Parser, Subcommand};
use rust_decimal::Decimal;
use thiserror::Error;

const INITIAL_TAX_LOT_ID: u64 = 1;

/// Represents the command line arguments
/// 
/// `selection_algo`: Determines how the tax lots are sold. Options: fifo, hifo
#[derive(Parser)]
pub struct TaxLotOpts {
    #[clap(subcommand)]
    selection_algo: SelectionAlgorithm,
}

/// Central enum for errors that can occur when processing tax lots.
#[derive(Debug, Error)]
pub enum TaxLotError {
    #[error("Could not parse date. Format: YYYY-mm-DD")]
    DateParseError(#[from] ParseError),
    #[error("Could not parse lot operation. {0} field does not exist")]
    FieldDoesntExist(String),
    #[error("Could not parse Lot Type. Options: buy, sell")]
    ParseLotTypeError,
    #[error("Could not parse Decimal")]
    DecimalParseError(#[from] rust_decimal::Error),
    #[error("Overflow occurred while {0}")]
    DecimalOverflow(String),
    #[error("Underflow occurred while {0}")]
    DecimalUnderflow(String),
    #[error("Could not parse price: price cannot be negative")]
    NegativePrice,
    #[error("Could not parse quantity: quantity cannot be negative")]
    NegativeQuantity,
}

/// Represents the selection algorithm for how the tax lots are sold.
/// 
/// fifo: tax lot that is bought first is also sold first
/// hifo: tax lot with the highest price is sold first.
#[derive(Debug, Subcommand, Clone, Copy)]
pub enum SelectionAlgorithm {
    #[clap(name = "fifo")]
    Fifo,

    #[clap(name = "hifo")]
    Hifo,
}

/// Represents the type of operation that can be applied to the tax lots.
/// 
/// Buy: create a new tax lot if no date currently exists or merge with existing tax lot.
/// Sell: Deduct the shares from the tax lots according to the selection algorithm.
#[derive(Debug, Eq, PartialEq)]
pub enum LotType {
    Buy,
    Sell,
}

impl FromStr for LotType {
    type Err = TaxLotError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "buy" => Ok(LotType::Buy),
            "sell" => Ok(LotType::Sell),
            _ => Err(TaxLotError::ParseLotTypeError),
        }
    }
}

/// Represents an operation that can be applied to the tax lots. These lot operations
/// are parsed from stdin.
#[derive(Debug)]
struct LotOperation {
    date: NaiveDate,
    lot_type: LotType,
    price: Decimal,
    quantity: Decimal,
}

impl FromStr for LotOperation {
    type Err = TaxLotError;

    fn from_str(s: &str) -> Result<Self, TaxLotError> {
        let parts: Vec<&str> = s.split(',').collect();
        let date = NaiveDate::parse_from_str(
            LotOperation::get_field_from_parts(&parts, 0, "Date".to_string())?,
            "%Y-%m-%d",
        )?;
        let lot_type = LotType::from_str(LotOperation::get_field_from_parts(&parts, 1, "Lot Type".to_string())?)?;
        let price = Decimal::from_str(LotOperation::get_field_from_parts(&parts, 2, "Price".to_string())?)?;
        if price <= Decimal::ZERO {
            return Err(TaxLotError::NegativePrice);
        }
        let quantity =
            Decimal::from_str(LotOperation::get_field_from_parts(&parts, 3, "Quantity".to_string())?)?;
        if quantity <= Decimal::ZERO {
            return Err(TaxLotError::NegativeQuantity);
        }

        Ok(LotOperation {
            date,
            lot_type,
            price,
            quantity,
        })
    }
}

impl LotOperation {
    /// Create a new lot from a lot operation. A new lot should be created when the `LotCollection` 
    /// does not have a lot for the date of the `LotOperation`.
    fn create_new_lot(self, id_generator: &AtomicU64, selection_algo: SelectionAlgorithm) -> Lot {
        Lot {
            id: id_generator.fetch_add(1, Ordering::SeqCst),
            date: self.date,
            price: self.price,
            quantity: self.quantity,
            selection_algo,
        }
    }

    /// Returns a `&str` from the vector of string slices according to the given index. Performs error
    /// checking to validate that the field exists. 
    fn get_field_from_parts<'a>(parts: &'a Vec<&str>, index: usize, field_name: String) -> Result<&'a str, TaxLotError> {
        match parts.get(index) {
            Some(field) => Ok(field),
            None => Err(TaxLotError::FieldDoesntExist(field_name))
        }
    }
}

/// Checked addition operation that maps an `Option` to a `Result` in case the operation
/// overflows.
fn checked_add(left: Decimal, right: Decimal) -> Result<Decimal, TaxLotError> {
    match left.checked_add(right) {
        Some(result) => Ok(result),
        None => Err(TaxLotError::DecimalOverflow("adding".to_string()))
    }
}

/// Checked multiplication operation that maps an `Option` to a `Result` in case the operation
/// overflows.
fn checked_mul(left: Decimal, right: Decimal) -> Result<Decimal, TaxLotError> {
    match left.checked_mul(right) {
        Some(result) => Ok(result),
        None => Err(TaxLotError::DecimalOverflow("multiplying".to_string()))
    }
}

/// Checked division operation that maps an `Option` to a `Result` in case the operation
/// underflows.
fn checked_div(left: Decimal, right: Decimal) -> Result<Decimal, TaxLotError> {
    match left.checked_div(right) {
        Some(result) => Ok(result),
        None => Err(TaxLotError::DecimalUnderflow("dividing".to_string()))
    }
}

/// Checked subtraction operation that maps an `Option` to a `Result` in case the operation
/// underflows.
fn checked_sub(left: Decimal, right: Decimal) -> Result<Decimal, TaxLotError> {
    match left.checked_sub(right) {
        Some(result) => Ok(result),
        None => Err(TaxLotError::DecimalUnderflow("subtracting".to_string()))
    }
}

/// Represents a TaxLot that can be sold. `LotOperation`s can be merged if they have been bought on the
/// same date.
#[derive(Debug)]
struct Lot {
    id: u64,
    date: NaiveDate,
    price: Decimal,
    quantity: Decimal,
    selection_algo: SelectionAlgorithm,
}

impl Display for Lot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display price with two decimals and quantity with 8 decimals.
        write!(
            f,
            "{},{},{:.2},{:.8}",
            self.id, self.date, self.price, self.quantity
        )
    }
}

impl Lot {
    /// "Merge" takes a lot operation, verifies that the dates are the same,
    /// computes the aggregate quantity, and then computes the weighted average
    /// price.
    fn merge(&mut self, lot_operation: LotOperation) -> Result<(), TaxLotError> {
        // Verify that the dates are the same, otherwise this is an invalid operation.
        assert!(lot_operation.date == self.date);

        // Verify that the operation is a buy operation
        assert!(lot_operation.lot_type == LotType::Buy);

        let left = checked_mul(self.price, self.quantity)?;
        let right = checked_mul(lot_operation.price, lot_operation.quantity)?;

        self.quantity = checked_add(self.quantity, lot_operation.quantity)?;

        let total = checked_add(left, right)?;
        self.price = checked_div(total, self.quantity)?;

        Ok(())
    }
}

impl Ord for Lot {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match &self.selection_algo {
            SelectionAlgorithm::Fifo => self.date.cmp(&other.date),
            // Reverse the comparison for `hifo` so that the lots are sorted from highest -> lowest.
            SelectionAlgorithm::Hifo => other.price.cmp(&self.price),
        }
    }
}

impl PartialOrd for Lot {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(match &self.selection_algo {
            SelectionAlgorithm::Fifo => self.date.cmp(&other.date),
            // Reverse the comparison for `hifo` so that the lots are sorted from highest -> lowest.
            SelectionAlgorithm::Hifo => other.price.cmp(&self.price),
        })
    }
}

impl PartialEq for Lot {
    fn eq(&self, other: &Self) -> bool {
        match self.selection_algo {
            SelectionAlgorithm::Fifo => self.date == other.date,
            SelectionAlgorithm::Hifo => self.price == other.price,
        }
    }
}

impl Eq for Lot {}

/// Represents a collection of tax lots. These lots can be sold, added to, or merged with an existing lot.
/// 
/// A `VecDeque` is used for efficient access to the "first" item, where the first item is dictated by the
/// `selection_algorithm`. If we're using `fifo`, the `lot_queue` is sorted by date and the first item will
/// be the oldest tax lot. If we're using `hifo`, the `lot_queue` is sorted by price and the first item will
/// be the highest price tax lot.
/// 
/// Buy Operation: Worst case O(N) to find lot with the same date, when the `selection_algo` is `hifo`. When the `selection_algo` is `fifo`, this is improved to O(1).
/// Sell Operation: Worst case (N) to sell all lots.
struct LotCollection {
    // Keeps a sorted queue according to the `selection_algorithm`.
    lot_queue: VecDeque<Lot>,

    // Generates ids for tax lots starting at 1.
    id_generator: AtomicU64,

    // Determines how the tax lots are sorted in the `lot_queue`.
    selection_algorithm: SelectionAlgorithm,
}

impl LotCollection {
    fn new(selection_algorithm: SelectionAlgorithm) -> Self {
        LotCollection {
            lot_queue: VecDeque::new(),
            id_generator: AtomicU64::new(INITIAL_TAX_LOT_ID),
            selection_algorithm,
        }
    }

    /// Applies a `buy` or `sell` lot operation to the lot collection.
    fn apply_lot_operation(&mut self, lot_operation: LotOperation) -> Result<(), TaxLotError> {
        match lot_operation.lot_type {
            LotType::Buy => self.buy(lot_operation),
            LotType::Sell => self.sell(lot_operation),
        }
    }

    /// Gets a lot from the lot collection according to the date.
    /// If the lot collection is sorted by date, we can just check
    /// the back of the queue to determine if a lot with the same date exists.
    fn get_lot(&mut self, date: &NaiveDate) -> Option<&mut Lot> {
        match self.selection_algorithm {
            SelectionAlgorithm::Fifo => {
                // If the selection algorithm is fifo, we can just check the back of the queue to
                // determine if a lot with the same date already exists
                if let Some(lot) = self.lot_queue.back_mut() {
                    if &lot.date == date {
                        return Some(lot);
                    }
                }

                return None;
            }
            SelectionAlgorithm::Hifo => {
                // We must search the whole queue to determine if a lot with the same date already exists
                return self.lot_queue.iter_mut().find(|existing_lot| &existing_lot.date == date);
            }
        }
    }

    /// Buy creates a new tax lot if there is no tax lot with the `lot_operation` date.
    /// Buy merges `lot_operation` with an existing lot if the `lot_collection` already
    /// has a `lot` with the specified date.
    fn buy(&mut self, lot_operation: LotOperation) -> Result<(), TaxLotError> {
        match self.get_lot(&lot_operation.date)
        {
            Some(existing_lot) => {
                // merge with an existing lot since the `lot_collection` already has a lot
                // for this date.
                existing_lot.merge(lot_operation)?;
            }
            None => {
                // create a new lot since `lot_collection` does not have a lot for this date.
                let new_lot =
                    lot_operation.create_new_lot(&self.id_generator, self.selection_algorithm);
                self.lot_queue.push_back(new_lot);
                self.lot_queue.make_contiguous().sort();
            }
        }

        Ok(())
    }

    /// Sell deducts "shares" from tax lots according to the `selection_algorithm`. Since `lot_queue` is sorted
    /// according to the `selection_algorithm`, this just needs to pop tax lots off of the queue and deduct
    /// shares from each lot until there are no more tax lots or we have sold the number of shares specified.
    fn sell(&mut self, lot_operation: LotOperation) -> Result<(), TaxLotError> {
        let mut quantity_sold = lot_operation.quantity;

        while quantity_sold > Decimal::ZERO {
            if let Some(lot) = self.lot_queue.front_mut() {
                let new_quantity = checked_sub(lot.quantity, quantity_sold)?;
                if new_quantity > Decimal::ZERO {
                    lot.quantity = new_quantity;
                    quantity_sold = Decimal::ZERO;
                } else {
                    quantity_sold = checked_sub(quantity_sold, lot.quantity)?;
                    self.lot_queue.pop_front();
                }
            } else {
                // We have run out of lots to sell, break out of the loop.
                break;
            }
        }

        Ok(())
    }
}

fn main() {
    let TaxLotOpts { selection_algo } = TaxLotOpts::parse();

    let mut lot_collection = LotCollection::new(selection_algo);

    // Process each line from stdin
    let lines = io::stdin().lines();
    for line in lines {
        match line {
            Ok(line) => {
                if let Err(e) = process_lot_operation(line.as_str(), &mut lot_collection) {
                    eprintln!("{e}");
                    process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error reading from stdin: {e}");
                process::exit(1);
            }
        }
    }

    while !lot_collection.lot_queue.is_empty() {
        if let Some(lot) = lot_collection.lot_queue.pop_front() {
            println!("{lot}");
        }
    }
}

fn process_lot_operation(op: &str, lot_collection: &mut LotCollection) -> Result<(), TaxLotError> {
    let lot_operation = LotOperation::from_str(op)?;
    lot_collection.apply_lot_operation(lot_operation)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::NaiveDate;
    use rust_decimal::{prelude::FromPrimitive, Decimal};

    use crate::{LotCollection, LotOperation, SelectionAlgorithm, TaxLotError, Lot};

    fn get_by_date<'a>(lot_collection: &'a LotCollection, date: &str) -> Result<&'a Lot, TaxLotError> {
        let naive_date = NaiveDate::from_str(date)?;
        Ok(lot_collection
            .lot_queue
            .iter()
            .find(|lot| lot.date == naive_date)
            .expect("No date found"))
    }

    #[test]
    fn test_parse_lot_operation() -> Result<(), TaxLotError> {
        LotOperation::from_str("2021-01-01,Buy,10000.00,1.00000000").expect("Failed to parse valid lot operation");
        LotOperation::from_str("2021-01-01,sell,10000.00,1.00000000").expect("Failed to parse valid lot operation");
        LotOperation::from_str("2021-01-01,sell,1,4").expect("Failed to parse valid lot operation");

        // invalid date
        LotOperation::from_str("2021-13-01,buy,10000.00,1.00000000").expect_err("Successfully parsed an invalid date");

        // invalid lot type
        LotOperation::from_str("2021-01-01,invalid,10000.00,1.00000000").expect_err("Successfully parsed an invalid lot type");

        // invalid price
        LotOperation::from_str("2021-01-01,buy,-10000.00,1.00000000").expect_err("Successfully parsed an invalid price");
        LotOperation::from_str("2021-01-01,buy,0.0,1.00000000").expect_err("Successfully parsed an invalid price");
        LotOperation::from_str("2021-01-01,buy,invalid,1.00000000").expect_err("Successfully parsed an invalid price");

        // invalid quantity
        LotOperation::from_str("2021-01-01,buy,10000.00,-1.00000000").expect_err("Successfully parsed an invalid quantity");
        LotOperation::from_str("2021-01-01,buy,10000.00,invalid").expect_err("Successfully parsed an invalid quantity");
        LotOperation::from_str("2021-01-01,buy,10000.00,0").expect_err("Successfully parsed an invalid quantity");

        // no quantity
        LotOperation::from_str("2021-01-01,buy,10000.00").expect_err("Successfully parsed lot opration with no quantity");

        // no price
        LotOperation::from_str("2021-01-01,buy").expect_err("Successfully parsed lot operation with no price");

        // no type
        LotOperation::from_str("2021-01-01").expect_err("Successfully parsed lot operation with no lot type");

        // no date
        LotOperation::from_str("").expect_err("Successfully parsed lot operation with no date");
        Ok(())
    }

    #[test]
    fn test_lot_displays_proper_formatting() -> Result<(), TaxLotError> {
        let lot = Lot {
            date: NaiveDate::from_str("2021-01-01")?,
            id: 1,
            price: Decimal::from_f64(10000.0).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.0).expect("Failed to parse quantity"),
            selection_algo: SelectionAlgorithm::Fifo,
        };

        let lot_string = lot.to_string();

        assert_eq!(lot_string, "1,2021-01-01,10000.00,1.00000000");

        Ok(())
    }

    #[test]
    fn test_merge_uses_weighted_average() -> Result<(), TaxLotError> {
        let mut lot = Lot {
            date: NaiveDate::from_str("2021-01-01")?,
            id: 1,
            price: Decimal::from_f64(10000.0).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.0).expect("Failed to parse quantity"),
            selection_algo: SelectionAlgorithm::Fifo,
        };

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(20000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(3.00000000).expect("Failed to parse quantity"),
        };

        lot.merge(lot_operation)?;

        assert_eq!(lot.id, 1);
        assert_eq!(lot.price, Decimal::from_f64(17500.0).expect("Failed to parse price"));
        assert_eq!(lot.quantity, Decimal::from_f64(4.0).expect("Failed to parse quantity"));

        Ok(())
    }

    #[test]
    fn test_buy_creates_new_lot() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Fifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(10000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-02")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(20000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(2.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        assert_eq!(lot_collection.lot_queue.len(), 2);
        let lot1 = get_by_date(&lot_collection, "2021-01-01")?;
        let lot2 = get_by_date(&lot_collection, "2021-01-02")?;
        assert_eq!(lot1.id, 1);
        assert_eq!(lot2.id, 2);
        assert_eq!(lot1.price, Decimal::from_f64(10000.00).expect("Failed to parse price"));
        assert_eq!(lot2.price, Decimal::from_f64(20000.00).expect("Failed to parse price"));
        assert_eq!(lot1.quantity, Decimal::from_f64(1.00000000).expect("Failed to parse quantity"));
        assert_eq!(lot2.quantity, Decimal::from_f64(2.00000000).expect("Failed to parse quantity"));

        Ok(())
    }

    #[test]
    fn test_buy_merges_with_existing_lot() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Fifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(10000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(20000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(3.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        assert_eq!(lot_collection.lot_queue.len(), 1);
        let lot1 = get_by_date(&lot_collection, "2021-01-01")?;
        assert_eq!(lot1.id, 1);
        assert_eq!(lot1.price, Decimal::from_f64(17500.00).expect("Failed to parse price"));
        assert_eq!(lot1.quantity, Decimal::from_f64(4.00000000).expect("Failed to parse quantity"));

        Ok(())
    }

    #[test]
    fn test_sell_deducts_only_lot() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Fifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(10000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-02-01")?,
            lot_type: crate::LotType::Sell,
            price: Decimal::from_f64(5000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(0.50000000).expect("Failed to parse quantity"),
        };
        lot_collection.sell(lot_operation)?;

        assert_eq!(lot_collection.lot_queue.len(), 1);
        let lot1 = get_by_date(&lot_collection, "2021-01-01")?;
        assert_eq!(lot1.price, Decimal::from_f64(10000.00).expect("Failed to parse price"));
        assert_eq!(lot1.quantity, Decimal::from_f64(0.50000000).expect("Failed to parse quantity"));

        Ok(())
    }

    #[test]
    fn test_sell_deducts_from_multiple_lots_fifo() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Fifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(10000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-02")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(20000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(3.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-03")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(15000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(10.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-02-01")?,
            lot_type: crate::LotType::Sell,
            price: Decimal::from_f64(5000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(7.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.sell(lot_operation)?;

        assert_eq!(lot_collection.lot_queue.len(), 1);
        let lot1 = get_by_date(&lot_collection, "2021-01-03")?;
        assert_eq!(lot1.price, Decimal::from_f64(15000.00).expect("Failed to parse price"));
        assert_eq!(lot1.quantity, Decimal::from_f64(7.00000000).expect("Failed to parse quantity"));

        Ok(())
    }

    #[test]
    fn test_sell_deducts_from_multiple_lots_hifo() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Hifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(10000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-02")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(20000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(3.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-03")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(15000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(10.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-02-01")?,
            lot_type: crate::LotType::Sell,
            price: Decimal::from_f64(5000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(7.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.sell(lot_operation)?;

        assert_eq!(lot_collection.lot_queue.len(), 2);

        let lot1 = get_by_date(&lot_collection, "2021-01-03")?;
        assert_eq!(lot1.price, Decimal::from_f64(15000.00).expect("Failed to parse price"));
        assert_eq!(lot1.quantity, Decimal::from_f64(6.00000000).expect("Failed to parse quantity"));

        let lot2 = get_by_date(&lot_collection, "2021-01-01")?;
        assert_eq!(lot2.price, Decimal::from_f64(10000.00).expect("Failed to parse price"));
        assert_eq!(lot2.quantity, Decimal::from_f64(1.00000000).expect("Failed to parse quantity"));

        Ok(())
    }

    #[test]
    fn test_sell_runs_out_of_lots() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Hifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-01")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(10000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(1.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-02")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(20000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(3.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-01-03")?,
            lot_type: crate::LotType::Buy,
            price: Decimal::from_f64(15000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(10.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.buy(lot_operation)?;

        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-02-01")?,
            lot_type: crate::LotType::Sell,
            price: Decimal::from_f64(5000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(15.00000000).expect("Failed to parse quantity"),
        };
        lot_collection.sell(lot_operation)?;

        // We have sold all of the lots, because the sell specified 15 shares and we have only bought (1+3+10)=14 shares
        assert_eq!(lot_collection.lot_queue.len(), 0);

        Ok(())
    }

    #[test]
    fn test_sell_with_no_lots() -> Result<(), TaxLotError> {
        let selection_algo = SelectionAlgorithm::Hifo;
        let mut lot_collection = LotCollection::new(selection_algo);
        let lot_operation = LotOperation {
            date: NaiveDate::from_str("2021-02-01")?,
            lot_type: crate::LotType::Sell,
            price: Decimal::from_f64(5000.00).expect("Failed to parse price"),
            quantity: Decimal::from_f64(15.00000000).expect("Failed to parse quantity"),
        };

        // The sell operation does not fail if there's no tax lots to sell, it will return success without changing the lot collection
        lot_collection.sell(lot_operation)?;
        assert_eq!(lot_collection.lot_queue.len(), 0);

        Ok(())
    }
}
