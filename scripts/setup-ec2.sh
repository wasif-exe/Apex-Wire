#!/bin/bash
set -euo pipefail

echo "=== Apex-Wire V2 EC2 Setup ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root: sudo bash $0"
    exit 1
fi

echo "[1/8] Detecting instance..."
TOKEN=$(curl -s -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 60" 2>/dev/null || true)
if [ -n "$TOKEN" ]; then
    ITYPE=$(curl -s -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/instance-type 2>/dev/null || echo "unknown")
    AZ=$(curl -s -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/placement/availability-zone 2>/dev/null || echo "unknown")
    echo "  Instance: $ITYPE | AZ: $AZ"
else
    echo "  Not on EC2 or IMDSv2 blocked"
fi

echo "[2/8] System packages..."
if command -v apt-get &>/dev/null; then
    apt-get update -qq
    apt-get install -y -qq build-essential linux-headers-$(uname -r) curl git clang llvm libelf-dev
elif command -v yum &>/dev/null; then
    yum install -y -q gcc gcc-c++ make kernel-devel-$(uname -r) git clang llvm elfutils-libelf-devel
elif command -v pacman &>/dev/null; then
    pacman -Syu --noconfirm base-devel linux-headers git clang llvm
fi

echo "[3/8] Rust toolchain..."
if ! command -v cargo &>/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
rustup default stable
echo "  $(rustc --version)"

echo "[4/8] Kernel limits..."
cat >> /etc/security/limits.conf << 'LIM'
* soft memlock unlimited
* hard memlock unlimited
* soft nofile 1048576
* hard nofile 1048576
LIM
ulimit -l unlimited
ulimit -n 1048576
echo "  memlock=$(ulimit -l) nofile=$(ulimit -n)"

echo "[5/8] Huge pages..."
HP=$(cat /proc/sys/vm/nr_hugepages)
if [ "$HP" -lt 64 ]; then
    echo 64 > /proc/sys/vm/nr_hugepages
    echo "  HugePages: $HP -> $(cat /proc/sys/vm/nr_hugepages)"
else
    echo "  HugePages: $HP (already sufficient)"
fi

echo "[6/8] CPU governor..."
for gov in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    echo performance > "$gov" 2>/dev/null || true
done
echo "  Governor: $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo 'N/A')"

echo "[7/8] Network tuning..."
NIC=$(ip -br link show | grep -v lo | grep -v docker | head -1 | awk '{print $1}')
if [ -n "$NIC" ]; then
    ethtool -G "$NIC" rx 4096 tx 4096 2>/dev/null || true
    ethtool -C "$NIC" adaptive-rx on adaptive-tx on 2>/dev/null || true
    ethtool -K "$NIC" gro on tso on 2>/dev/null || true
    echo "  NIC=$NIC rings tuned"
    echo "  Driver: $(ethtool -i "$NIC" 2>/dev/null | grep driver || echo 'unknown')"
else
    echo "  No NIC found"
fi
sysctl -w net.core.rmem_max=33554432 2>/dev/null || true
sysctl -w net.core.wmem_max=33554432 2>/dev/null || true
sysctl -w net.core.netdev_max_backlog=250000 2>/dev/null || true

echo "[8/8] XDP support check..."
if zcat /proc/config.gz 2>/dev/null | grep -q "CONFIG_XDP_SOCKETS=y" || \
   grep -q "CONFIG_XDP_SOCKETS=y" /boot/config-$(uname -r) 2>/dev/null; then
    echo "  XDP_SOCKETS: enabled"
else
    echo "  XDP_SOCKETS: unknown (may still work)"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "  1. Clone Apex-Wire:"
echo "     git clone https://github.com/wasif-exe/Apex-Wire.git"
echo "     cd Apex-Wire"
echo ""
echo "  2. Build:"
echo "     cargo build --release"
echo ""
echo "  3. Run benchmark:"
echo "     sudo bash -c 'ulimit -l unlimited; taskset -c 2 ./target/release/aw_v2_final'"
echo ""
echo "  4. For AF_XDP test (if ENA supports it):"
echo "     sudo bash -c 'ulimit -l unlimited; taskset -c 2 ./target/release/aw_v3_benchmark'"
