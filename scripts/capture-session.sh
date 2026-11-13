#!/usr/bin/env bash
# Leads through capture session 2: one HX Edit action per capture. For each step it says what to
# do, captures usbmon while you do it, and keeps only the pedal's traffic in captures/.
#
# Run from the repo root on the host, with HX Edit open in the VM and the pedal passed through:
#   scripts/capture-session.sh            all steps
#   scripts/capture-session.sh 12         from step 12 on (after an interruption)
#
# Everything happens on presets 03A-03C ("New Preset" places); your own presets 01A-02B are not
# touched. Nothing here writes to the pedal: you do every action yourself in HX Edit.

set -euo pipefail

STEPS=(
  "07-select-03A|Kies in HX Edit preset 03A (New Preset)."
  "08-add-amp|Klik in 03A op een leeg blok op pad A en kies Amp > Revv Gen Red."
  "09-change-model|Verander het model van dat amp-blok in Cali Rectifire."
  "10-add-delay|Voeg na de amp een Simple Delay toe."
  "11-move-block|Sleep de delay naar een plek vóór de amp."
  "12-toggle-switch|Zet bij de Simple Delay de schakelaar Trails om (aan of uit)."
  "13-add-cab-mic|Voeg een cab toe (4x12 Cali V30) en kies bij Mic een andere microfoon."
  "14-remove-block|Verwijder de Simple Delay."
  "15-rename-snapshot|Hernoem snapshot 1 naar Test."
  "16-rename-preset|Hernoem preset 03A naar NX Test."
  "17-save-preset|Sla 03A op (Ctrl+S of Save)."
  "18-export-preset|Exporteer 03A naar een .hlx-bestand op het bureaublad van de VM."
  "19-import-to-03B|Importeer Revv_Home.hlx naar plek 03B. Dit is het Assign-aan-preset-X-verzoek."
  "20-copy-paste-03C|Kopieer 01A (Recto DM) en plak hem op 03C."
  "21-clear-03C|Wis preset 03C (Clear of Initialize)."
  "22-global-tuner|Global Settings: zet Tuner Reference van 440 naar 441 Hz en weer terug naar 440."
  "23-backup|Maak in HX Edit een back-up (Backup) naar het bureaublad van de VM. Wacht tot hij klaar is."
  "24-footswitch-mode|Geen HX Edit: trap op de Stomp FS1 en FS2 samen in, kijk of de modus wisselt, en weer terug."
)

cd "$(git rev-parse --show-toplevel)"
mkdir -p captures local/raw
log="local/capture-session-2.log"
start_at="${1:-0}"

pedal() {
  for d in /sys/bus/usb/devices/*; do
    [[ -f $d/idVendor && $(<"$d/idVendor") == 0e41 && $(<"$d/idProduct") == 4246 ]] || continue
    echo "$(<"$d/busnum") $(<"$d/devnum")"
    return
  done
  return 1
}

for step in "${STEPS[@]}"; do
  label="${step%%|*}"
  action="${step#*|}"
  number="${label%%-*}"
  (( 10#$number < 10#$start_at )) && continue

  read -r bus dev < <(pedal) || { echo "HX Stomp niet gevonden op de USB-bus; stop." >&2; exit 1; }
  raw="local/raw/$label.pcapng"
  echo
  echo "== $label"
  echo "   $action"
  read -rp "   Enter om de opname te starten (s = overslaan) " answer
  [[ $answer == s ]] && { echo "$(date -Is) $label overgeslagen" >>"$log"; continue; }

  sg wireshark -c "dumpcap -q -i usbmon$bus -w $raw" &
  sleep 2
  read -rp "   Opname loopt. Doe de handeling, wacht tot HX Edit klaar is en druk dan op Enter " _
  sleep 1
  pkill -x dumpcap || true
  wait 2>/dev/null || true

  tshark -r "$raw" -Y "usb.device_address == $dev" -w "captures/$label.pcapng" 2>/dev/null
  read -rp "   Opmerking (wat zag je, mag leeg) " note
  echo "$(date -Is) $label device $dev: $action | $note" >>"$log"
  echo "   opgeslagen: captures/$label.pcapng"
done

echo
echo "Klaar. Log: $log"
