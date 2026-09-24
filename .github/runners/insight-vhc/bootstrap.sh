#!/usr/bin/env bash
set -euxo pipefail

RUNNER_VERSION=2.337.0
RUNNER_SHA=70920811a4f8ad4328818682bca5c6469c1c942fab52448868071d0063816613

export DEBIAN_FRONTEND=noninteractive
echo 'DPkg::Lock::Timeout "600";' > /etc/apt/apt.conf.d/90-gha-lock-timeout
systemctl disable --now unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer
apt-get update
apt-get install -y ca-certificates curl gnupg jq git unzip build-essential \
  pkg-config libssl-dev protobuf-compiler cmake libcurl4-openssl-dev acl \
  software-properties-common

# Ubuntu 24.04 ships git 2.43, whose partial-clone behaviour fails git-cli-proxy's
# promisor tests. The hosted images take the same PPA.
add-apt-repository -y ppa:git-core/ppa
apt-get update
apt-get install -y git
git --version

id -u runner >/dev/null 2>&1 || useradd -m -s /bin/bash runner
echo 'runner ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/90-runner
chmod 0440 /etc/sudoers.d/90-runner

root_disk=$(lsblk -no PKNAME "$(findmnt -no SOURCE /)")
for _ in $(seq 60); do
  data_disk=$(lsblk -dnro NAME,TYPE | awk '$2=="disk"{print $1}' | grep -v "^${root_disk}$" | head -1 || true)
  [ -n "$data_disk" ] && break
  sleep 5
done
[ -n "$data_disk" ]
blkid "/dev/${data_disk}" >/dev/null 2>&1 || mkfs.ext4 -F -L gha-data "/dev/${data_disk}"
mkdir -p /srv/gha
grep -q LABEL=gha-data /etc/fstab || \
  echo 'LABEL=gha-data /srv/gha ext4 defaults,noatime 0 2' >> /etc/fstab
mount -a
install -d /srv/gha/docker
install -d -o runner -g runner /srv/gha/work

install -d /etc/docker
echo '{"data-root": "/srv/gha/docker"}' > /etc/docker/daemon.json
install -m0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
echo "deb [arch=amd64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
  > /etc/apt/sources.list.d/docker.list
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
systemctl enable --now docker
usermod -aG docker runner

YQ_VERSION=v4.53.6
YQ_SHA256=c5f056448f973ae7d39b5401949648a78f2dc1947d6a8eb65be60d5c504b9385
curl -fsSL -o /tmp/yq \
  "https://github.com/mikefarah/yq/releases/download/${YQ_VERSION}/yq_linux_amd64"
echo "${YQ_SHA256}  /tmp/yq" | sha256sum -c -
install -m 0755 /tmp/yq /usr/local/bin/yq
rm -f /tmp/yq

# Present on the GitHub-hosted images and assumed by lanes that install neither:
# helm by the chart-contract workflows, gh by anything shelling out to the API.
# From release tarballs rather than the vendors' apt repositories: baltocdn.com
# rejects the TLS handshake from this network.
HELM_VERSION=v3.22.0
# Pinned rather than fetched: the checksum published beside the tarball shares
# its host, so it proves nothing against that host being wrong.
HELM_SHA256=1e4ab49e429626cf6c6958d914248b78c9730803c2751b87627e171dc800e7bb
GH_VERSION=2.101.0
GH_SHA256=9bca2d1c16825f109907a23307628a2f0698fbf99662b73a5cf0b020293072b8

curl -fsSL -o /tmp/gh.tgz \
  "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_amd64.tar.gz"
echo "${GH_SHA256}  /tmp/gh.tgz" | sha256sum -c -
tar -xzf /tmp/gh.tgz -C /tmp
install -m 0755 "/tmp/gh_${GH_VERSION}_linux_amd64/bin/gh" /usr/local/bin/gh
rm -rf /tmp/gh.tgz "/tmp/gh_${GH_VERSION}_linux_amd64"

# helm ships no binary asset on GitHub, only a detached signature, so the
# tarball comes from get.helm.sh and is checked against the digest above.
curl -fsSL -o /tmp/helm.tgz \
  "https://get.helm.sh/helm-${HELM_VERSION}-linux-amd64.tar.gz"
echo "${HELM_SHA256}  /tmp/helm.tgz" | sha256sum -c -
tar -xzf /tmp/helm.tgz -C /tmp
install -m 0755 /tmp/linux-amd64/helm /usr/local/bin/helm
rm -rf /tmp/helm.tgz /tmp/linux-amd64

helm version --short
gh --version

mkdir -p /opt/actions-runner
cd /opt/actions-runner
curl -fsSL -o runner.tar.gz \
  "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz"
echo "${RUNNER_SHA}  runner.tar.gz" | sha256sum -c -
tar xzf runner.tar.gz
rm -f runner.tar.gz
./bin/installdependencies.sh
chown -R runner:runner /opt/actions-runner

# A container job runs as root over the mounted work tree, so one killed before
# its cleanup leaves root-owned paths that the next host job's checkout cannot
# remove. The runner reads this hook from .env at startup and runs it on the
# host before every job, container jobs included.
install -m 0755 /dev/stdin /usr/local/sbin/gha-job-started.sh <<'HOOK'
#!/usr/bin/env bash
set -uo pipefail
work="/srv/gha/work"
[ -d "$work" ] || exit 0
if sudo find "$work" -xdev ! -user "$(id -un)" -print -quit 2>/dev/null | grep -q .; then
  echo "job-started hook: reclaiming $work"
  sudo chown -R "$(id -u):$(id -g)" "$work" || true
else
  echo "job-started hook: $work clean"
fi
# A reused work tree keeps whatever remote-tracking refs an earlier job fetched,
# and a later checkout neither refreshes nor removes them, so anything reading
# origin/<branch> compares against a base that has fallen behind. A runner that
# starts from an empty tree has none; drop them so this one matches.
find "$work" -maxdepth 4 -type d -name .git -print0 2>/dev/null |
  while IFS= read -r -d '' gitdir; do
    dropped=0
    for ref in $(git --git-dir="$gitdir" for-each-ref --format='%(refname)' refs/remotes/ 2>/dev/null); do
      git --git-dir="$gitdir" update-ref -d "$ref" 2>/dev/null && dropped=$((dropped + 1))
    done
    if [ "$dropped" -gt 0 ]; then
      echo "job-started hook: dropped $dropped remote-tracking ref(s) in $gitdir"
    fi
  done
# The runner fails the job when this hook exits non-zero, so nothing above may
# decide the exit status.
exit 0
HOOK
grep -q ACTIONS_RUNNER_HOOK_JOB_STARTED /opt/actions-runner/.env 2>/dev/null || \
  echo 'ACTIONS_RUNNER_HOOK_JOB_STARTED=/usr/local/sbin/gha-job-started.sh' >> /opt/actions-runner/.env
chown runner:runner /opt/actions-runner/.env

set +x
. /etc/gha-runner/register.env
if [ -z "$RUNNER_TOKEN" ]; then
  echo "gha-bootstrap: no registration token, runner left unregistered"
  exit 1
fi
runuser -u runner -- ./config.sh --unattended --replace \
  --url "$GITHUB_URL" --token "$RUNNER_TOKEN" \
  --name "$RUNNER_NAME" --labels "$RUNNER_LABELS" --work /srv/gha/work
rm -f /etc/gha-runner/register.env
set -x

./svc.sh install runner
./svc.sh start
echo "gha-bootstrap: runner $RUNNER_NAME registered at $GITHUB_URL"
