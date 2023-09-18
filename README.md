# Tax Lot Task

## Building

### Dev Environment
For a reproducible build environment, a nix flake has been provided which will install rust and cargo.
To use the nix environment, [install nix](https://zero-to-nix.com/start/install) with flakes turned on, and then run:

```
nix develop
```

Otherwise, just install `cargo` and `rustc` (but there are no guarantees on reproducibility).

### Building the executable
To build the executable:

```
cargo build
```

The built executable will be available at `./target/debug/taxlot` or can be run using `cargo run`.

Example invocation:

```
echo -e '2021-01-01,buy,10000.00,1.00000000\n2021-01-02,buy,20000.00,1.00000000\n2021-02-01,sell,20000.00,1.5000' | ./target/debug/taxlot hifo
```

## Running the tests
To run the unit tests:

```
cargo test
```

To run the bash integration tests:

```
./integration_test.sh
```

## Design
This application supports two main operations: `buy` and `sell`. Buy must support efficient creation of new lots and merging with existing lots.
Sell must support efficient traversal through the lot collection according to the tax lot selection algorithm. 
When thinking about data structures to use to support these operations, it appeared that there was a tradeoff: optimize for `buy` or `sell`.

For example, it is possible to store the tax lots in a vector that is sorted order according to their date. This would make `buy` time complexity O(1) for average and worse case (assuming tax lot operations are processed in ascending order). 
For `sell`, if the selection algorithm was `fifo`, then time complexity is O(1) to get the "next" item and O(N) to sell all lots. However, if the selection algorithm was `hifo`, then to `sell` all lots, it would be O(N^2) since it is possible the lot collection needs to be traversed N times to find the "next" tax lot because the lots are not stored in order of price.

Instead, I elected to store the tax lots in a sorted queue based on the selection algorithm. For `fifo`, the tax lots are stored sorted by their date. For `hifo`, the tax lots are stored sorted by their price. This makes each `buy` operation O(N) for `hifo`, since we need to search the whole list to figure out if we need to make a new tax lot. For `fifo`, this can be optimized to O(1) (assuming tax lot operations are processed in ascending order) since the list is stored sorted by the date. Each
`sell` operation is now worst case time complexity O(N) to sell all the tax lots, since we just need to pop all the elements of the queue in order.

Given that there is only a single tax lot per day, optimizing for `sell` seemed like the smart choice since it made that operation much more efficient, while only requiring O(N) time complexity for `buy`, which isn't that bad considering the lot collection
structure shouldn't have too many items in it (unless it is processing data for decades).


## Production Improvements
This tax lot application is just a simple command line application that takes input from stdin and prints the remaining tax lots to stdout. 
For production, there's a number of things that can be improved:

 - Make thread safe. `LotCollection` is not currently a thread safe data structure. We could add a lock over each `Lot` in the `LotCollection` to allow parallel access to lots on different days.
 - Use a database. This is a simple application that doesn't require persistence, but a real application would need to store these tax lots long term, and not re-process them each time. Using a database
 is a good fit for this. The database table for tax lots could be efficiently indexed by both `price` and `date`, for efficient lookup for both `fifo` and `hifo`.
 - More modular code structure. The code structure is currently just a single file with a module for tests. This is fine for a small application, but if we want to share `LotCollection` with a different application or crate,
 this code structure makes it difficult to do that. Creating a module that allows other crates to import this struct would be good for reusability.
 - Scale and performance tests. Currently I have only included unit tests that test the basic functional requirements and check for error conditions, as well as a ~150 tax lot operation integration test. This works for testing
 functionality, but it doesn't tell us much about performance or how this code operates at scale. Creating a "at-scale" test where there are 10000+ operations would be good to test for performance (even if this is an unrealistic scenario).
 - Audit log. When `LotOperation`s are consumed, in the current code they just disappear/merge into the existing `Lot` on the same date. This is functionally correct, but for production it would likely be nice to have an audit log to be able to 
 determine what operations happened to each lot and when they happened. This is mainly useful for debugging, but I assume there are compliance reasons to do this too. Tracing this to a log or emitting a telemetry event that can later be queried
 would also be a good thing to do.

