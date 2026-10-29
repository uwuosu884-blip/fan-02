#!/bin/sh
# Installed by `omen15-fan service install` into
# /usr/lib/systemd/system-sleep/omen15-fan (systemd calls this automatically
# before/after suspend/hibernate — see systemd-sleep(8), no enabling needed).
#
# Why this exists: after a lid close/open (or any suspend/resume), the
# ec_sys debugfs node and/or the omen15-fan service can stop responding
# until the kernel module is reloaded and the service is restarted. This
# hook does that automatically so you don't have to do it by hand every
# time you close the lid.

set -e

case "$1/$2" in
  pre/*)
    # Hand control back to the BIOS and stop the watchdog loop before the
    # EC gets suspended out from under it.
    systemctl stop omen15-fan.service 2>/dev/null || true
    ;;
  post/*)
    # Give the EC a moment to finish resuming, then make sure ec_sys is
    # loaded with write support (debugfs nodes can be re-probed across
    # resume) and restart the service.
    sleep 1
    modprobe -r ec_sys 2>/dev/null || true
    sleep 1
    modprobe ec_sys write_support=1 2>/dev/null || true
    sleep 1
    systemctl start omen15-fan.service 2>/dev/null || true
    ;;
esac

exit 0
