# Server RDP di prova: xrdp + Xfce (immagine di scottyhardy) con un utente fittizio proprio.
# L'immagine di base ignora le variabili USERNAME/PASSWORD e crea solo "ubuntu/ubuntu": qui si aggiunge
# `pxtuser`, con una password che contiene un "-" (il tasto che cambia posto tra i layout di tastiera,
# utile per provare il layout). Solo credenziali di prova, solo su loopback.
FROM scottyhardy/docker-remote-desktop:latest
RUN useradd --create-home --shell /bin/bash --groups sudo pxtuser \
    && echo 'pxtuser:pxtpass-1234' | chpasswd

# `xclip` serve alle prove degli appunti condivisi: legge e scrive gli appunti della sessione grafica.
RUN apt-get update && apt-get install -y --no-install-recommends xclip \
    && rm -rf /var/lib/apt/lists/*
