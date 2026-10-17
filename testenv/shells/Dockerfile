# Server di prova con più shell (bash, zsh, tcsh, ash) per il tracciamento della directory.
# `bashuser` può usare `sudo` per passare agli altri utenti (prova della re-iniezione dopo il cambio utente).
# SOLO sviluppo: utenti e password fittizi (password "test"), porta esposta solo su loopback.
FROM alpine:3.20
RUN apk add --no-cache openssh bash zsh tcsh sudo \
 && ssh-keygen -A \
 && for pair in bashuser:/bin/bash zshuser:/bin/zsh tcshuser:/bin/tcsh ashuser:/bin/ash; do \
      u="${pair%%:*}"; s="${pair##*:}"; \
      adduser -D -s "$s" "$u" && echo "$u:test" | chpasswd; \
    done
# `sudo su - altro_utente` senza password (come un amministratore SAP che passa a <sid>adm).
RUN echo 'bashuser ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/bashuser && chmod 440 /etc/sudoers.d/bashuser
COPY sshd_config /etc/ssh/sshd_config
EXPOSE 2222
CMD ["/usr/sbin/sshd", "-D", "-e"]
