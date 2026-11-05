# Testing in the official Sailfish OS emulator (without the SDK IDE)

We use Jolla's emulator image (`releases.sailfishos.org/sdk/emulators/`, newest: 5.1.0.11) in
VirtualBox 7.2, set up by hand.

## One-time setup

```sh
mkdir -p ~/SailfishOS/emulator && cd ~/SailfishOS/emulator
curl -fLO https://releases.sailfishos.org/sdk/emulators/Sailfish_OS-5.1.0.11-Sailfish_SDK_Emulator.7z
7z x Sailfish_OS-5.1.0.11-Sailfish_SDK_Emulator.7z            # -> sailfishos.vdi

# SSH: the image reads /etc/ssh/authorized_keys/<user>/authorized_keys from a vboxsf share "ssh"
mkdir -p share/ssh/defaultuser share/ssh/root share/config
ssh-keygen -t ed25519 -N "" -f share/id_ed25519
for u in defaultuser root; do cp share/id_ed25519.pub share/ssh/$u/authorized_keys; done

VM="Sailfish OS Emulator 5.1.0.11"
VBoxManage createvm --name "$VM" --ostype Linux26 --basefolder $PWD/vm --register
VBoxManage modifyvm "$VM" --memory 2048 --cpus 2 --ioapic on --pae on --rtc-use-utc on \
  --graphicscontroller vboxvga --vram 128 --mouse usbtablet --usb-ohci on --audio-driver none \
  --nic1 nat --nictype1 virtio --natpf1 "ssh,tcp,127.0.0.1,2223,,22"
VBoxManage storagectl "$VM" --name SATA --add sata --portcount 1 --hostiocache on
VBoxManage storageattach "$VM" --storagectl SATA --port 0 --device 0 --type hdd --medium $PWD/sailfishos.vdi
VBoxManage sharedfolder add "$VM" --name ssh --hostpath $PWD/share/ssh
VBoxManage sharedfolder add "$VM" --name config --hostpath $PWD/share/config

# Device model like the SDK's (read by the image from the "config" share at boot)
printf '[desktop/sailfish/silica]\ntheme_pixel_ratio=1.25\n' > share/config/device-model.txt
printf 'QT_QPA_EGLFS_WIDTH=768\nQT_QPA_EGLFS_HEIGHT=1280\nQT_QPA_EGLFS_PHYSICAL_WIDTH=62\nQT_QPA_EGLFS_PHYSICAL_HEIGHT=103\n' \
  > share/config/65-emul-wayland-ui-scale.conf

THREESEA_VM_TYPE=separate scripts/emulator-start.sh   # visible window; closing it keeps the VM running

# once booted (ssh -p 2223 -i share/id_ed25519 root@127.0.0.1): the image lacks Sailfish Secrets
pkcon refresh && pkcon -y install sailfishsecretsdaemon sailfishsecretsdaemon-secretsplugins-default libsailfishsecrets
```

The image also lacks commhistory-daemon, whose notification categories give new-message
notifications their tone and vibration on a phone. For notification tests, write the two we use
(contents as in commhistory-daemon's RPM):

```sh
cd /usr/share/lipstick/notificationcategories
printf 'app_icon=icon-lock-sms\nx-nemo-display-on=true\nx-nemo-priority=120\nx-nemo-feedback=chat_exists\n' > x-nemo.messaging.im.conf
printf 'app_icon=icon-lock-sms\ntransient=true\nx-nemo-display-on=true\nx-nemo-priority=120\nx-nemo-feedback=chat\n' > x-nemo.messaging.im.preview.conf
```

The emulator plays no sound; see which feedback lipstick asks for with
`dbus-monitor --system "destination=com.nokia.NonGraphicFeedback1.Backend"` (as root).

Notes:
- The kernel has no driver for VirtualBox's default NIC → `virtio`; graphics need `vboxvga`
  (vboxvideo DRM). `vmsvga` gives a black screen.
- **Portrait-native screen** (like the SDK, see `sailfish-qtcreator`
  `src/libs/sfdk/vboxvirtualmachine.cpp` / `emulator.cpp`): the SDK sets `CustomVideoMode1`,
  `GUI/LastGuestSizeHint`, `GUI/AutoresizeGuest=false`, and writes `device-model.txt` (dconf) and
  `65-emul-wayland-ui-scale.conf` into the `config` share. The guest's vboxvideo driver only
  takes the size hint while it initializes and the GUI frontend is what sends it, so headless
  we send `setvideomodehint` repeatedly during the first seconds of boot
  (`scripts/emulator-start.sh`). Use 768 px width: 720 gives a garbled picture (pitch alignment).
  Without this, the screen is 1024×768 landscape and apps come up rotated.
- After each boot, Sailfish Secrets asks to confirm access to our key once (the emulator has no
  device lock code; on a phone with a lock code the collection unlocks with the device).
- Jolla Settings may not start from the app grid in this setup; `nohup jolla-settings &` over
  SSH works. Its device-lock switch does not react in the emulator.
- **Translucency is rendered too weak**: the emulator's software renderer blends translucent
  colours with alpha squared (measured: 0.5 → ~0.23). The i486 build compensates
  (`backend.emulator`, `Fmt.alpha()` = sqrt) so overlays look as on a device; device builds use
  the values measured on real Jolla Messages screenshots (bubbles 12 % / 6 % white).
- **Video and QtMultimedia need a package** the image lacks:
  `pkcon -y install qt5-qtmultimedia-plugin-mediaservice-gstmediaplayer` (as root). It decodes in
  software through Qt's renderer, so a phone's `droideglsink` route is still its own test, and voice
  then takes QMediaPlayer's route instead of the core's PCM fallback. For a test video add
  `gstreamer1.0-tools` and write one with `gst-launch-1.0` (`avenc_mpeg4`, `avenc_aac`, `mp4mux`).
- `pkill` patterns: the process name is truncated to 15 chars (`harbour-threese`).
- **Going back a page does not work with the edge swipe here**: a swipe from the left edge
  minimizes the app (the home screen appears) instead of popping the page, whatever its
  length. Drive forward only and start the app again (`pkill` + launch) to get back to the
  chat list; the app always opens there.

## Daily use

```sh
scripts/sdk-build.sh i486 && scripts/emulator-deploy.sh
scripts/emulator.py shot /tmp/screen.png      # screenshot (portrait)
scripts/emulator.py tap 300 420               # tap
scripts/emulator.py swipe 384 300 384 520     # e.g. open a pulley menu
scripts/emulator.py swipe 384 1250 384 400    # app grid
scripts/emulator.py type "text"; scripts/emulator.py key enter
ssh -p 2223 -i ~/SailfishOS/emulator/share/id_ed25519 root@127.0.0.1 journalctl -f | grep threesea
```

Call media engine (Full, ADR 0012): `scripts/test-callengine.sh` builds `tests/callengine` (two
webrtcbin engines calling each other) in the SDK image and runs it in the emulator as
`defaultuser` (PulseAudio session); it prints both SDPs and `connected`, exit code 0 on success.
With `--build-only` it stops after the build (what CI and `scripts/run-sdk-tests.sh` do).
Sound (calls, voice messages): the VM needs VirtualBox's PulseAudio driver with input and
output, set while it is off:
`VBoxManage modifyvm "Sailfish OS Emulator 5.1.0.11" --audio-driver pulse --audio-enabled on --audio-in on --audio-out on`.
A headless VM gets a window later with `VirtualBoxVM --startvm "<name>" --separate`.
Ringtones and message tones stay silent even with sound on: the image's default profile names
`ring-1.wav`, which it doesn't contain (only `/usr/share/sounds/jolla-ringtones/stereo/*.ogg`),
so ngfd reports "Resource not found". Check the requests with dbus-monitor (system bus) instead.


## Demo data (emulator builds only)

The emulator's install uses the project's test ID, and the only contact there that answers is
ECHOECHO — so a chat list to work against cannot be staged by hand. The i486 build can fill
itself with made-up chats:

```sh
scripts/emulator-demo.sh add        # or: remove
```

There is no entry in the app for it, on purpose: what goes on a phone must carry nothing of
this (CLAUDE.md §10). It is asked for over D-Bus, and all three parts exist only in an
emulator build — the core command (Cargo feature `demo`), the D-Bus method
(`qmake CONFIG+=demo`, `DBusInterface::demoData`) and the handler in `main.cpp`. A package for
a phone has no trace: `strings` over the binary and `grep` over the QML and the `.qm` files
find nothing.

What it writes (`core/src/demo.rs`, command `demoData`, Cargo feature `demo`, switched on for
i486 only in `scripts/build-core.sh`):

- contacts `DEMO0001`… with names, two with a profile picture, every verification level, and one
  unknown contact without a name (`DEMO0009`, whose chat offers the spam report; it doesn't exist on
  the server, so a report on it is refused);
- a pinned chat, an archived one, a muted one, one with a message still pending and one that
  failed, and — on purpose — an older unread chat *and* a newer one, which is what the cover's
  "who is waiting" needs;
- a chat with a long history (400 messages over 90 days by default: `{"messages": n}`) for
  scrolling, paging and the quick scroll;
- 40 favourite GIFs (about 90 MB in the store), for the emoji panel with many of them;
- a chat with one of everything: pictures, a GIF with a transparent part, a voice message (a
  real Ogg Opus tone), a document, a video whose download failed, a location, a quote, an edited message, a deleted one,
  reactions and a starred message;
- a group with a picture, a mention of us and unread messages; a notes group (only ourselves);
  a distribution list.

Nothing is sent and no blob is fetched: everything is written straight into the store, and the
pictures and the voice file are generated in memory. Removing takes the demo contacts, their
chats, the demo group, the notes group, the list and their media away again.
