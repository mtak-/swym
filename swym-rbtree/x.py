#!/usr/bin/env python

import os
import sys

if sys.argv[1] == 'test':
    prefix = "RUST_TEST_THREADS=1"
    suffix = '--features stats'
elif sys.argv[1] == 'bench':
    prefix = 'RUSTFLAGS="$RUSTFLAGS -Ctarget-cpu=native"'
    suffix = ''
else:
    prefix = ''
    suffix = ''
result = os.system(prefix + ' cargo +nightly ' +
                   sys.argv[1] + ' ' + suffix + ' ' + ' '.join(sys.argv[2:]))
sys.exit(0 if result == 0 else -1)
