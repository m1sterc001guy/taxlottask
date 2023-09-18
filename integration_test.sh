#!/bin/bash

## This test feeds our application some example data and verifies that the return code
## is expected.

cargo build

# Verify simple success case
input_data="2021-01-01,buy,10000.00,1.00000000\n2021-02-01,sell,20000.00,0.50000000"
echo -e "$input_data" | ./target/debug/taxlot fifo

return_code="$?"
if [ $return_code -ne 0 ]; then
    echo "Error: expected return code 0"
    exit 1
fi

# Verify returns exit code 1 when it cannot parse the input
input_data="2021-01-01,invalid,10000.00,1.00000000\n2021-02-01,sell,20000.00,0.50000000"
echo -e "$input_data" | ./target/debug/taxlot fifo

return_code="$?"
if [ $return_code -ne 1 ]; then
    echo "Error: expected return code 1"
    exit 1
fi

# Verify returns exit code 1 when it cannot parse the input
input_data="2021-01-01,buy,10000.00,1.00000000\n2021-02-01,sell,20000.00,0.5000asas"
echo -e "$input_data" | ./target/debug/taxlot fifo

return_code="$?"
if [ $return_code -ne 1 ]; then
    echo "Error: expected return code 1"
    exit 1
fi

# Verify more complex success case with `hifo`
cat test_data.txt | ./target/debug/taxlot hifo
return_code="$?"
if [ $return_code -ne 0 ]; then
    echo "Error: expected return code 0"
    exit 1
fi
echo "HIFO half Year test successful"

# Verify more complex success case with `fifo`
cat test_data.txt | ./target/debug/taxlot fifo
return_code="$?"
if [ $return_code -ne 0 ]; then
    echo "Error: expected return code 0"
    exit 1
fi
echo "FIFO half Year test successful"

echo "Successfully finished integration test"