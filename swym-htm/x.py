#!/usr/bin/env python

import os
import sys

if sys.argv[1] == 'test' or sys.argv[1] == 'doc':
    prefix = 'RUST_TEST_THREADS=1 RUSTFLAGS="$RUSTFLAGS -Ctarget-feature=+rtm -Ctarget-cpu=native"'
    suffix = '--features rtm'
elif sys.argv[1] == 'bench':
    prefix = 'RUSTFLAGS="$RUSTFLAGS -Ctarget-feature=+rtm -Ctarget-cpu=native"'
    suffix = '--features rtm'
else:
    prefix = ''
    suffix = ''
result = os.system(prefix + ' cargo ' + sys.argv[1] + ' ' + suffix + ' ' + ' '.join(sys.argv[2:]))
sys.exit(0 if result == 0 else -1)
