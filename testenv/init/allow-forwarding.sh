#!/bin/sh
# Solo per il test dei jump host: il server di prova deve consentire direct-tcpip.
sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /config/sshd/sshd_config
