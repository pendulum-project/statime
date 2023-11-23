#!/usr/bin/env bash

set -eo pipefail
set -x

case $1 in
  post-install|post-upgrade)
      # Ensure users are created
      id statime

      # Ensure deamon is present
      echo -e "\nSTATIME HELP OUTPUT:"
      /usr/bin/statime --help

      # # Ensure that the systemd service is not running by default
      # ! systemctl is-active statime.service --quiet
    ;;
esac
