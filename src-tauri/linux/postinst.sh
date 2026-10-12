#!/bin/sh
udevadm control --reload >/dev/null 2>&1 || true
udevadm trigger >/dev/null 2>&1 || true
