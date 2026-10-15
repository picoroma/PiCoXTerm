#!/bin/sh
# Solo per i test: il server di prova deve consentire direct-tcpip (jump host) e non penalizzare
# gli accessi falliti ripetuti, che i test provocano di proposito.
sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /config/sshd/sshd_config
grep -q '^PerSourcePenalties' /config/sshd/sshd_config || echo 'PerSourcePenalties no' >> /config/sshd/sshd_config
