#!/usr/bin/env nu

# Open the Surface Pro 11 speaker path before anything else can.
#
# AudioReach builds its graph lazily, when a PCM is first opened, and fixes the
# front-end to back-end connection at that moment.  Whoever opens PCM1 first
# decides what the machine sounds like for the rest of the boot: get there
# after the desktop's sound server, with the WSA route still half written, and
# the amplifiers spend the session on a bus the DSP never configured, which is
# audible as pops and nothing else.
#
# What a pass proves is the digital path: the PCM ran, and neither amplifier's
# DAPM endpoint stayed down.  It says nothing about whether sound comes out.
#
# Ported from ooaklee/linux-surface-pro-11-oe, scripts/sp11-enable-wsa-routing.sh,
# minus its machine-card rebind between attempts: a rebind reloads host topology
# and graph state without resetting either amplifier, so it is a second chance
# rather than a different one.

# Two WSA8845 amplifiers, addresses 0 and 1 on SoundWire controller 1.
const SLAVES = [
  "/sys/bus/soundwire/devices/sdw:1:0:0217:0204:00:0/status"
  "/sys/bus/soundwire/devices/sdw:1:0:0217:0204:00:1/status"
]

# The ceilings x1e80100.c imposes with snd_soc_limit_volume, "to reduce the risk
# of speaker damage until we have active speaker protection in place".  Raising
# either is not merely ignored: the rejected cset aborts the sequence.
const PA_VOLUME = "6"
const DIGITAL_VOLUME = "81"

def log [message: string] {
  print $"wsa-routing: ($message)"
}

def poll [what: string, timeout: duration, check: closure] {
  mut waited = 0sec
  while $waited < $timeout {
    if (do $check) { return }
    sleep 1sec
    $waited = $waited + 1sec
  }
  error make {msg: $"($what) after ($timeout)"}
}

def card-registered [card: string] {
  (^aplay -l | complete | get stdout | str contains $card)
}

# The controls arrive with the topology, well after the card itself.
def topology-loaded [card: string] {
  (^amixer -c $card cget "name='WSA_CODEC_DMA_RX_0 Audio Mixer MultiMedia2'"
    | complete | get exit_code) == 0
}

def amplifiers-attached [] {
  $SLAVES | all {|status|
    ($status | path exists) and ((open $status | str trim) == "Attached")
  }
}

# ALSA writes the single word "closed" here when no substream owns the PCM.
def pcm-closed [card: string] {
  let status = $"/proc/asound/($card)/pcm1p/sub0/status"
  (not ($status | path exists)) or ((open $status | str trim) == "closed")
}

def cset [card: string, control: string, value: string] {
  let result = (^amixer -c $card cset $"name='($control)'" $value | complete)
  if $result.exit_code != 0 {
    error make {msg: $"($control) = ($value) rejected: ($result.stderr | str trim)"}
  }
}

def apply-route [card: string] {
  # Cleared first, so the mixer connects once, at the end, rather than part-way
  # through the route being written.
  cset $card "WSA_CODEC_DMA_RX_0 Audio Mixer MultiMedia2" "0"

  cset $card "WSA WSA RX0 MUX" "AIF1_PB"
  cset $card "WSA WSA RX1 MUX" "AIF1_PB"
  cset $card "WSA WSA_RX0 INP0" "RX0"
  cset $card "WSA WSA_RX1 INP0" "RX1"
  cset $card "WSA WSA_COMP1 Switch" "1"
  cset $card "WSA WSA_COMP2 Switch" "1"
  cset $card "WSA WSA_RX0 Digital Volume" $DIGITAL_VOLUME
  cset $card "WSA WSA_RX1 Digital Volume" $DIGITAL_VOLUME
  cset $card "WSA WSA_RX0 Digital Mute" "0"
  cset $card "WSA WSA_RX1 Digital Mute" "0"

  for amplifier in ["SpkrLeft" "SpkrRight"] {
    cset $card $"($amplifier) COMP Switch" "1"
    cset $card $"($amplifier) BOOST Switch" "1"
    cset $card $"($amplifier) DAC Switch" "1"
    cset $card $"($amplifier) PBR Switch" "1"
    cset $card $"($amplifier) VISENSE Switch" "0"
    cset $card $"($amplifier) WSA MODE" "Speaker"
    cset $card $"($amplifier) PA Volume" $PA_VOLUME
  }

  cset $card "WSA_CODEC_DMA_RX_0 Audio Mixer MultiMedia2" "1"
}

# Bounded because a graph that fails to open takes aplay down with it, and this
# runs between the boot and the greeter.
def probe-graph [card: string] {
  let result = (^timeout 15 aplay -q -D $"hw:($card),1" -t raw -f S16_LE -r 48000 -c 4 -d 2 /dev/zero
    | complete)
  if $result.exit_code != 0 {
    log $"silent probe failed: ($result.stderr | str trim)"
    return false
  }

  let widgets = "/sys/devices/platform/sound/WSA Playback/dapm_widget"
  if not ($widgets | path exists) { return true }

  let dapm = (open $widgets)
  ["SpkrLeft SPKR: On" "SpkrRight SPKR: On"] | all {|endpoint|
    $dapm | str contains $endpoint
  }
}

# The card is passed in rather than hardcoded so that audio.nix, which also
# writes it into the PipeWire sink, holds the only copy.
# Every budget below is generous by an order of magnitude, because the greeter
# waits on this unit: the card probes about six seconds into boot, its controls
# arrive with it as part of the same topology load, and both amplifiers have
# already attached by then.
def main [card: string] {
  poll "no sound card" 20sec { card-registered $card }
  poll "no topology controls" 10sec { topology-loaded $card }
  poll "amplifiers never attached to SoundWire" 10sec { amplifiers-attached }

  for attempt in 1..2 {
    poll $"PCM1 still held on attempt ($attempt)" 5sec { pcm-closed $card }
    apply-route $card

    if (probe-graph $card) {
      log "route applied, graph open, both amplifiers powered"
      return
    }

    # Closing the failed PCM drops the graph references it took, so the next
    # open starts clean and there is nothing else to undo.
    log "retrying with a fresh PCM"
    sleep 3sec
  }

  error make {msg: "speaker route never came up; look for 100100[0-6] and qcom-apm in the kernel log"}
}
