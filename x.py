#!/usr/bin/env python

import os
import sys

if sys.argv[1] == 'test':
    prefix = "RUST_TEST_THREADS=1"
    suffix = '--features debug-alloc,stats'
elif sys.argv[1] == 'bench':
    prefix = 'RUSTFLAGS="$RUSTFLAGS -Ctarget-cpu=native"'
    suffix = ''
else:
    prefix = ''
    suffix = ''
os.system(prefix + ' cargo ' + sys.argv[1] + ' ' + suffix + ' ' + ' '.join(sys.argv[2:]))
