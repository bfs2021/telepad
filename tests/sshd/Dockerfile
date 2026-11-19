# Throwaway SSH server for Telepad's file window tests (made-up data only).
# AUTHORIZED_KEY: the public key to log in with; NO_SFTP=1 removes the SFTP subsystem.
FROM alpine:3.20
RUN apk add --no-cache openssh openssh-sftp-server rsync \
 && ssh-keygen -A \
 && adduser -D -s /bin/sh telepad \
 && sed -i 's/^telepad:!:/telepad:*:/' /etc/shadow \
 && sed -i 's/^#\?AllowTcpForwarding.*/AllowTcpForwarding yes/' /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 22
CMD ["/entrypoint.sh"]
