"""The run-ledger sweep's pure core (connector-health spec §3.2).

`lib/sweep.sh` gathers the inputs and performs the inserts; every decision about
what one tick records lives here, over values, so the rules are testable without
a connection.
"""
