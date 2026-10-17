// Android host adapter for the unmodified, pinned GPL-3.0 mihomo module.
package mobile

import (
	"errors"
	"fmt"
	"net/netip"
	"sync"
	"sync/atomic"
	"syscall"

	"github.com/metacubex/mihomo/component/dialer"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub"
	LC "github.com/metacubex/mihomo/listener/config"
	"github.com/metacubex/mihomo/listener/sing_tun"
	"github.com/metacubex/mihomo/tunnel"
)

// An error-returning callback also clears exceptions raised by the Java host.
type Protector interface {
	Protect(fd int32) (bool, error)
}
type protectorHolder struct{ value Protector }

var (
	lifecycle sync.Mutex
	allowed atomic.Pointer[protectorHolder]
	tun *sing_tun.Listener
	started bool
)

func init() {
	// Installed once: replacing the global hook during active dials would race.
	dialer.DefaultSocketHook = func(network, address string, conn syscall.RawConn) error {
		p := allowed.Load()
		if p == nil { return nil }
		var protectErr error
		if err := conn.Control(func(fd uintptr) {
			ok, err := p.value.Protect(int32(fd))
			if err != nil { protectErr = err } else if !ok {
				protectErr = errors.New("Android rejected socket protection")
			}
		}); err != nil { return err }
		return protectErr
	}
}

func Start(home, raw string) error {
	lifecycle.Lock()
	defer lifecycle.Unlock()
	if started { return errors.New("core is already running") }
	C.SetHomeDir(home)
	if err := hub.Parse([]byte(raw)); err != nil { return err }
	started = true
	return nil
}

func Reload(raw string) error {
	lifecycle.Lock()
	defer lifecycle.Unlock()
	if !started { return errors.New("core is not running") }
	// Rebind the controller if its port changes. The Android TUN is owned separately.
	return hub.Parse([]byte(raw))
}

func SetProtector(value Protector) {
	if value == nil { allowed.Store(nil) } else { allowed.Store(&protectorHolder{value}) }
}

// Java owns the original descriptor. sing_tun owns only this duplicate.
func StartTun(fd int32, ipv6 bool) error {
	lifecycle.Lock()
	defer lifecycle.Unlock()
	if !started || allowed.Load() == nil { return errors.New("core or socket protector is not ready") }
	if tun != nil { return errors.New("VPN tunnel already exists") }
	copyFD, err := syscall.Dup(int(fd))
	if err != nil { return err }
	v6 := []netip.Prefix{}
	if ipv6 { v6 = append(v6, netip.MustParsePrefix("fdfe:dcba:9876::1/126")) }
	options := LC.Tun{
		Enable: true, Device: "ClashOfRust", Stack: C.TunGvisor,
		AutoRoute: false, AutoDetectInterface: false, MTU: 1500,
		Inet4Address: []netip.Prefix{netip.MustParsePrefix("198.18.0.1/30")},
		Inet6Address: v6, DNSHijack: []string{"0.0.0.0:53", "[::]:53"},
		FileDescriptor: copyFD,
	}
	// Fixed valid options reach tunNew, which takes ownership of copyFD.
	listener, err := sing_tun.New(options, tunnel.Tunnel)
	if err != nil { return fmt.Errorf("Android TUN: %w", err) }
	tun = listener
	return nil
}

func StopTun() error {
	lifecycle.Lock()
	defer lifecycle.Unlock()
	if tun == nil { return nil }
	err := tun.Close()
	tun = nil
	return err
}
