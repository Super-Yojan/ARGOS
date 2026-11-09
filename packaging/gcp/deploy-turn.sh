#!/bin/bash
# From a machine signed in to gcloud (your Mac): open the relay ports and
# install coturn on argos-host. Rerunnable.
#   bash packaging/gcp/deploy-turn.sh
set -euo pipefail
project=${ARGOS_GCP_PROJECT:-glid-app}
zone=${ARGOS_GCP_ZONE:-us-central1-a}
vm=${ARGOS_GCP_VM:-argos-host}
here=$(cd "$(dirname "$0")" && pwd)

if ! gcloud compute firewall-rules describe argos-turn --project="$project" >/dev/null 2>&1; then
    gcloud compute firewall-rules create argos-turn --project="$project" \
        --direction=INGRESS --action=ALLOW \
        --rules=udp:3478,tcp:3478,tcp:5349,udp:49160-49200 \
        --target-tags=argos-turn --source-ranges=0.0.0.0/0 \
        --description='ARGOS FPV relay (coturn)'
fi
gcloud compute instances add-tags "$vm" --project="$project" --zone="$zone" --tags=argos-turn
gcloud compute scp --project="$project" --zone="$zone" \
    "$here/install-turn.sh" "$here/refresh-router-tls.sh" "$vm":/tmp/
gcloud compute ssh "$vm" --project="$project" --zone="$zone" --command='
    set -e
    sudo install -o root -g root -m 755 /tmp/refresh-router-tls.sh /opt/argos/bin/refresh-router-tls.sh
    sudo bash /tmp/install-turn.sh'
echo "Credential: gcloud compute ssh $vm --project=$project --zone=$zone --command=\"sudo cat /etc/argos/turn.password\""
