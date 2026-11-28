# Assistant voice runtime

The Assistant starts this CPU-only Phonon process with private pipes. The
process loads the packaged Gradium Phonon 31M English Q8 checkpoint and its
default speaker embedding. The build enables the local tokenizer and omits
the Hub feature. Phonon receives only bounded reply text chosen for speech.

The pipe begins with `EV01`. Send one UTF-8 line per utterance. The process
streams mono 24 kHz signed 16-bit little-endian PCM. Each block starts with
its length as a little-endian unsigned 32-bit integer. A zero length ends
the utterance. Closing the pipes ends the process. The duration conditioner
sets the brisk default voice.

Build and verify from the workspace root, with build files on the workspace
build volume.

```sh
batch="$PWD/target/assistant-voice-implementation"
mkdir -p "$batch/pkg/"{sources,build,out}
(
  cd worktrees/assistant-voice-2026-11-27/enzimos-shell/voice-runtime
  SRCDEST="$batch/pkg/sources" BUILDDIR="$batch/pkg/build" \
    PKGDEST="$batch/pkg/out" makepkg
)
```

The package ships the exact model revision, voice, tokenizer and their
Apache 2.0 license. The runtime build records its exact Cargo dependencies
in `Cargo.lock`. The model is loaded from
`/usr/share/enzimos/voice/phonon-31m-en` on an installed system.
