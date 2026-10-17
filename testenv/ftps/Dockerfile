# Server FTPS **implicito** (TLS subito, porta 990) per le prove di PiCoXterm: ProFTPD con certificato
# autofirmato. SOLO sviluppo: utente fittizio test/test, porte esposte solo su loopback.
FROM alpine:3.20
RUN apk add --no-cache proftpd proftpd-mod_tls openssl \
 && adduser -D -h /home/test -s /bin/sh test \
 && echo "test:test" | chpasswd \
 && mkdir -p /etc/ssl/private /run/proftpd \
 && openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
      -keyout /etc/ssl/private/proftpd.pem -out /etc/ssl/private/proftpd.pem \
      -subj "/CN=localhost/O=PiCoXterm test/C=IT" \
 && mkdir -p /home/test/Documenti /home/test/Immagini \
 && echo "Benvenuto sul server FTPS di prova (TLS implicito)." > /home/test/LEGGIMI.txt \
 && echo "Relazione di prova" > /home/test/Documenti/relazione.txt \
 && printf 'nome;valore\nuno;1\ndue;2\n' > /home/test/Documenti/dati.csv \
 && head -c 300000 /dev/urandom > /home/test/Immagini/foto.bin \
 && chown -R test:test /home/test
COPY proftpd.conf /etc/proftpd/proftpd.conf
EXPOSE 990
CMD ["proftpd", "--nodaemon", "--config", "/etc/proftpd/proftpd.conf"]
