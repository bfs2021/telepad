#!/bin/sh
set -e
mkdir -p /home/telepad/.ssh
printf '%s\n' "$AUTHORIZED_KEY" > /home/telepad/.ssh/authorized_keys
chown -R telepad:telepad /home/telepad/.ssh
chmod 700 /home/telepad/.ssh
chmod 600 /home/telepad/.ssh/authorized_keys
if [ "$NO_SFTP" = 1 ]; then
  sed -i '/^Subsystem/d' /etc/ssh/sshd_config
fi
exec /usr/sbin/sshd -D -e
