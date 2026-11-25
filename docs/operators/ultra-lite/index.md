# Ultra Lite

Ultra Lite is the whole of Aspen in one program, `aspen-ul`, on one Linux machine: the API, the web
client, calls, file storage, and its own PostgreSQL, which it starts and stops itself. There is
nothing else to install or run. It suits a deployment of up to a few hundred people with a few dozen
active at once. It runs on one machine only: it cannot be spread across several.

**Ultra Lite has no horizontal scalability, no redundancy, and requires scheduled downtime for
upgrades and maintenance.** Ultra Lite is provided for ease of use and is intended for casual usage.
If this can't meet your use case, consider a full Aspen deployment instead. Ultra Lite deployments 
can migrate to a full Aspen deployment later.

| Page                              | What it covers                                                                    |
| --------------------------------- | --------------------------------------------------------------------------------- |
| This page                         | What the machine needs, and installing step by step                               |
| [Configuration](configuration.md) | Every setting of `aspen-ul.toml`                                                  |
| [Running it](operating.md)        | Commands, `doctor`, backups and restoring, upgrading, logs, and the data it keeps |
| [Leaving it](leaving.md)          | Moving to the full deployment, and moving the files to object storage             |

## What the machine needs

| Need      | What                                                                                                                                                              |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| System    | 64-bit Linux, x86-64 or ARM (a Raspberry Pi 5), with glibc 2.36 or later: Debian 12, Ubuntu 24.04, Raspberry Pi OS Bookworm, or newer. systemd to run it at boot. |
| Libraries | `libssl3` (OpenSSL 3), installed on those systems by default.                                                                                                     |
| Memory    | 1 GiB or more.                                                                                                                                                    |
| Disk      | A few GiB, and room for every file people upload and for backups. An SSD rather than an SD card.                                                                  |
| A domain  | A name of your own (`chat.example.org`), and `media.chat.example.org` for uploaded files, both pointing at the machine.                                           |
| Ports     | Reachable from the internet: TCP 443 and 80, UDP and TCP 40000 (calls), UDP and TCP 3478 and UDP 40100 to 40199 (file transfers in calls).                        |

Optional: `ffmpeg` and `ffprobe`, from the system's packages, for still pictures of shared videos.
SVGs are drawn by the renderer `aspen-ul` carries, with fonts of its own.

## Installing

1. **Get `aspen-ul`** for the machine's architecture (`uname -m` says which): `aspen-ul-x86_64` or
   `aspen-ul-aarch64` from a
   [release of Aspen](https://github.com/aspen-chat/aspen-chat-server/releases), one file holding
   everything, and `SHA256SUMS` beside it. Check it and name it `aspen-ul`:

   ```
   sha256sum --check --ignore-missing SHA256SUMS
   mv aspen-ul-$(uname -m) aspen-ul && chmod +x aspen-ul
   ```

   Or build it on an x86-64 Linux machine with the repository: `scripts/package_ul.py --arch x86_64`
   (or `--arch aarch64`) writes `target/ul/<arch>/aspen-ul`.

2. **Install it**, as root:

   ```
   sudo ./aspen-ul install
   ```

   This makes the user `aspen-ul`, its data directory `/var/lib/aspen-ul`, installs the program as
   `/usr/local/bin/aspen-ul`, and a systemd service, `aspen-ul`, started at boot.

3. **Point your names at the machine.** At your DNS provider, give `chat.example.org` and
   `media.chat.example.org` an `A` record with the machine's public IPv4 address, and an `AAAA`
   record with its IPv6 address if it has one.

4. **Open the ports.** A machine behind a home router needs each port in the table above forwarded
   to it. A firewall on the machine or at a hosting provider needs them open.

5. **Describe the deployment:**

   ```
   sudo aspen-ul init --domain chat.example.org --email you@example.org
   ```

   **The domain is for good.** Your accounts' passkeys and other deployments know the deployment by
   it, and it can never change; `init` asks you to type it again. `--email` is where Let's Encrypt
   writes about the certificate. A machine behind a router also needs
   `--announced-address <the router's public address>`, the address calls are offered at;
   `aspen-ul doctor` says which. See [Configuration](configuration.md) for the rest, and for a
   deployment behind a proxy of your own (`--tls off`).

6. **Start it:**

   ```
   sudo systemctl start aspen-ul
   ```

   The first start unpacks the program's PostgreSQL and web client, makes the database, and asks
   Let's Encrypt for a certificate, which takes a minute.

7. **Check it:**

   ```
   sudo aspen-ul doctor
   ```

   It checks the machine, the deployment, DNS, the router, and what the internet sees at your names,
   and says what to fix for each problem.

8. **Make yourself the administrator.** Create your account at `https://chat.example.org/register`,
   then:

   ```
   sudo aspen-ul admin grant <your username>
   ```

   To have creating an account take an invite from the start, give `init` `--invite-only`: it prints
   a link that creates the first account. See
   [The first administrator](../installing/7-first-administrator.md) for what the top role allows.

From then on, administrators manage everything else from the Administration Dashboard.
