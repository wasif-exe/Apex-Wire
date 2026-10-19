#!/bin/bash
set -euo pipefail

echo "=== Apex-Wire V2 AWS EC2 Deployment ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root: sudo $0"
    exit 1
fi

echo "[1/7] System prerequisites..."
apt-get update -qq || yum update -y -q || pacman -Syu --noconfirm
apt-get install -y -qq build-essential linux-headers-$(uname -r) || true

echo "[2/7] Kernel limits..."
echo "* soft memlock unlimited" >> /etc/security/limits.conf
echo "* hard memlock unlimited" >> /etc/security/limits.conf
ulimit -l unlimited

echo "[3/7] Huge pages (32 MB)..."
echo 16 > /proc/sys/vm/nr_hugepages
grep HugePages_Total /proc/meminfo

echo "[4/7] CPU governor (performance)..."
for cpu in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
    echo performance > "$cpu" 2>/dev/null || true
done

echo "[5/7] IRQ affinity (pin NIC to core 0)..."
NIC=$(ip -br link show | grep -v lo | grep -v docker | head -1 | awk '{print $1}')
if [ -n "$NIC" ]; then
    IRQS=$(grep "$NIC" /proc/interrupts | awk '{print $1}' | tr -d ':')
    for irq in $IRQS; do
        echo 1 > /proc/irq/"$irq"/smp_affinity 2>/dev/null || true
    done
    echo "  NIC=$NIC IRQs pinned to core 0"
else
    echo "  No NIC found, skipping IRQ pinning"
fi

echo "[6/7] VFIO (if dedicated NIC available)..."
if lspci | grep -qiE 'mlx5|ena|i40e|ixgbe'; then
    modprobe vfio-pci || true
    echo "  vfio-pci loaded"
else
    echo "  No XDP-capable NIC detected, VFIO skipped"
fi

echo "[7/7] Build Apex-Wire..."
cd "$(dirname "$0")/.."
cargo build --release
echo "  Build complete."

echo ""
echo "=== Deployment Complete ==="
echo "Run benchmark:"
echo "  taskset -c 2 ./target/release/aw_v2_final"
echo ""
echo "For AF_XDP with real NIC:"
echo "  sudo bash -c 'ulimit -l unlimited; taskset -c 2 ./target/release/aw_v3_benchmark'"
