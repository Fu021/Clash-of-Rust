package mobile

import (
	"errors"
	"net"
	"net/netip"
	"testing"

	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/tunnel"
)

type fixtureOwner struct {
	packages    map[string]int32
	destination string
	source      string
	protocol    int32
	sourcePort  int32
	targetPort  int32
	fail        bool
}

type fixtureApplicationProxy struct {
	C.Proxy
	name    string
	adapter C.ProxyAdapter
}

func (proxy *fixtureApplicationProxy) Name() string            { return proxy.name }
func (proxy *fixtureApplicationProxy) Adapter() C.ProxyAdapter { return proxy.adapter }

type fixtureApplicationSelector struct {
	C.ProxyAdapter
	proxies  []C.Proxy
	fallback C.Proxy
}

func (selector *fixtureApplicationSelector) Proxies() []C.Proxy     { return selector.proxies }
func (selector *fixtureApplicationSelector) EmptyFallback() C.Proxy { return selector.fallback }

func (owner *fixtureOwner) Owner(protocol int32, source string, sourcePort int32, destination string, destinationPort int32) (int32, error) {
	owner.source = source
	owner.destination = destination
	owner.protocol = protocol
	owner.sourcePort = sourcePort
	owner.targetPort = destinationPort
	if owner.fail {
		return -1, errors.New("fixture lookup failed")
	}
	return 10001, nil
}

func (owner *fixtureOwner) PackageUID(packageName string) (int32, error) {
	return owner.packages[packageName], nil
}

func TestApplicationIdentitiesRejectSharedUIDConflictsAndSkipRemovedPackages(t *testing.T) {
	SetConnectionOwner(&fixtureOwner{packages: map[string]int32{"org.example.first": 10001, "org.example.second": 10001}})
	defer SetConnectionOwner(nil)
	raw := "cor-application-rules:\n  schema: 1\n  routes:\n    - {package: org.example.first, target: DIRECT}\n    - {package: org.example.second, target: REJECT}\n"
	if _, err := parseApplicationRules(raw); err == nil {
		t.Fatal("conflicting shared UID routes were accepted")
	}
	raw = "cor-application-rules:\n  schema: 1\n  routes:\n    - {package: org.example.first, target: REJECT, reject_missing: true}\n    - {package: org.example.second, target: REJECT, reject_missing: true, missing_target: true}\n"
	if _, err := parseApplicationRules(raw); err == nil {
		t.Fatal("different missing-target semantics were accepted for a shared UID")
	}
	rules, err := parseApplicationRules("cor-application-rules:\n  schema: 1\n  routes:\n    - {package: org.example.removed, target: DIRECT}\n")
	if err != nil || len(rules.owners) != 0 {
		t.Fatalf("uninstalled package should not retain a UID: %v", err)
	}
}

func TestOwnerLookupUsesOriginalTupleAfterFakeIPMetadataWasRewritten(t *testing.T) {
	tunnel.SetMode(tunnel.Rule)
	owner := &fixtureOwner{packages: map[string]int32{"org.example.first": 10001}}
	SetConnectionOwner(owner)
	applicationRoutes.Store(&resolvedApplicationRules{owners: map[uint32]applicationRule{10001: {Package: "org.example.first", Target: "missing"}}})
	defer SetConnectionOwner(nil)
	defer applicationRoutes.Store(nil)
	metadata := &C.Metadata{
		NetWork: C.TCP, SrcIP: netip.MustParseAddr("198.18.0.1"), SrcPort: 40000,
		Host: "example.test", DstPort: 443,
		RawSrcAddr: &net.TCPAddr{IP: net.ParseIP("198.18.0.1"), Port: 40000},
		RawDstAddr: &net.TCPAddr{IP: net.ParseIP("198.18.0.23"), Port: 443},
	}
	if _, err := applicationPackage(metadata); err != nil {
		t.Fatal(err)
	}
	if owner.destination != "198.18.0.23" || owner.source != "198.18.0.1" {
		t.Fatal("the original tuple was lost")
	}
	if metadata.Uid != 10001 {
		t.Fatal("UID was not attached")
	}
	applicationRoutes.Store(&resolvedApplicationRules{owners: map[uint32]applicationRule{
		10001: {Package: "org.example.first", Target: "missing", RejectMissing: true},
	}})
	before := applicationMissingRejected.Load()
	if result, err := applicationPackage(metadata); err != nil || result != "cor-reject:org.example.first" {
		t.Fatalf("missing-target rejection lost its independent matching identity: %q %v", result, err)
	}
	if applicationMissingRejected.Load() != before+1 {
		t.Fatal("a real missing-target rejection did not reach background notification counters")
	}
	owner.packages["org.example.first"] = -1
	if result, err := applicationPackage(metadata); err == nil || result != "" {
		t.Fatalf("an uninstalled application's assignment survived UID reuse: %q %v", result, err)
	}
	owner.packages["org.example.first"] = 10002
	if result, err := applicationPackage(metadata); err == nil || result != "" {
		t.Fatalf("a reassigned UID inherited another application's assignment: %q %v", result, err)
	}
	owner.fail = true
	if _, err := applicationPackage(metadata); err == nil {
		t.Fatal("owner failure was silently accepted")
	}
}

func TestApplicationTargetNeverFallsThroughForUnsupportedUDP(t *testing.T) {
	for _, rejectMissing := range []bool{false, true} {
		rule := applicationRule{Target: "node", RejectMissing: rejectMissing}
		if got := applicationTarget(rule, true, false, true, false); got != "REJECT" {
			t.Fatalf("unsupported UDP routed to %q", got)
		}
		if got := applicationTarget(rule, true, false, false, false); got != "node" {
			t.Fatalf("TCP routed to %q", got)
		}
		for _, available := range []bool{false, true} {
			expected := ""
			if rejectMissing {
				expected = "REJECT"
			}
			if got := applicationTarget(rule, available, true, false, true); got != expected {
				t.Fatalf("missing target routed to %q, wanted %q", got, expected)
			}
		}
	}
}

func TestApplicationOwnerPreservesIPv6AndUDPOriginalTuples(t *testing.T) {
	tunnel.SetMode(tunnel.Rule)
	owner := &fixtureOwner{packages: map[string]int32{"org.example.first": 10001}}
	SetConnectionOwner(owner)
	applicationRoutes.Store(&resolvedApplicationRules{owners: map[uint32]applicationRule{
		10001: {Package: "org.example.first", Target: "DIRECT", RejectMissing: true},
	}})
	defer SetConnectionOwner(nil)
	defer applicationRoutes.Store(nil)
	for _, addresses := range [][2]string{
		{"198.18.0.1", "198.18.0.23"},
		{"fd00::1", "fdfe:dcba:9876::23"},
		{"::ffff:198.18.0.1", "::ffff:198.18.0.23"},
	} {
		for _, network := range []C.NetWork{C.TCP, C.UDP} {
			metadata := &C.Metadata{
				NetWork: network, SrcIP: netip.MustParseAddr("192.0.2.1"),
				DstIP: netip.MustParseAddr("203.0.113.1"), DstPort: 80,
				Host: "rewritten.example.test",
			}
			source := net.ParseIP(addresses[0])
			target := net.ParseIP(addresses[1])
			protocol := int32(6)
			if network == C.UDP {
				protocol = 17
				metadata.RawSrcAddr = &net.UDPAddr{IP: source, Port: 40001}
				metadata.RawDstAddr = &net.UDPAddr{IP: target, Port: 443}
			} else {
				metadata.RawSrcAddr = &net.TCPAddr{IP: source, Port: 40001}
				metadata.RawDstAddr = &net.TCPAddr{IP: target, Port: 443}
			}
			beforeRejected := applicationMissingRejected.Load()
			result, err := applicationPackage(metadata)
			if err != nil || result != "org.example.first" {
				t.Fatalf("original tuple did not match its application: %q %v", result, err)
			}
			if owner.source != source.String() || owner.destination != target.String() ||
				owner.protocol != protocol || owner.sourcePort != 40001 || owner.targetPort != 443 {
				t.Fatalf("rewritten metadata replaced the original tuple: %+v", owner)
			}
			if applicationMissingRejected.Load() != beforeRejected {
				t.Fatal("a valid original tuple was reported as a missing outbound")
			}
		}
	}
}

func TestMissingOrInvalidApplicationTupleFollowsConfiguration(t *testing.T) {
	tunnel.SetMode(tunnel.Rule)
	owner := &fixtureOwner{packages: map[string]int32{"org.example.first": 10001}}
	SetConnectionOwner(owner)
	applicationRoutes.Store(&resolvedApplicationRules{owners: map[uint32]applicationRule{
		10001: {Package: "org.example.first", Target: "missing", RejectMissing: true},
	}})
	defer SetConnectionOwner(nil)
	defer applicationRoutes.Store(nil)
	for _, raw := range []net.Addr{nil, &net.UnixAddr{Name: "invalid", Net: "unix"}} {
		metadata := &C.Metadata{NetWork: C.TCP, RawSrcAddr: raw, RawDstAddr: raw}
		beforeFailed := ownerFailures.Load()
		beforeRejected := applicationMissingRejected.Load()
		result, err := applicationPackage(metadata)
		if err == nil || result != "" || ownerFailures.Load() != beforeFailed+1 {
			t.Fatalf("an unidentifiable connection was assigned an application: %q %v", result, err)
		}
		if owner.source != "" || applicationMissingRejected.Load() != beforeRejected {
			t.Fatal("an invalid tuple reached the owner API or missing-target rejection")
		}
	}
}

func TestStaticMissingFallbackKeepsItsReasonSeparateFromExplicitReject(t *testing.T) {
	for _, reject := range []bool{true, false} {
		rule := applicationRule{Target: "REJECT", RejectMissing: reject, MissingTarget: true}
		expected := ""
		if reject {
			expected = "REJECT"
		}
		if target := applicationTarget(rule, true, false, false, true); target != expected {
			t.Fatalf("static missing fallback routed to %q, wanted %q", target, expected)
		}
	}
	if target := applicationTarget(applicationRule{Target: "REJECT"}, true, false, false, true); target != "REJECT" {
		t.Fatal("an explicit reject rule was mistaken for a missing target")
	}
}

func TestBuiltInOutboundsNeverReportMissingWithoutAnExplicitMissingMarker(t *testing.T) {
	tunnel.SetMode(tunnel.Rule)
	SetConnectionOwner(&fixtureOwner{packages: map[string]int32{"org.example.first": 10001}})
	defer SetConnectionOwner(nil)
	defer applicationRoutes.Store(nil)
	for _, target := range []string{"DIRECT", "REJECT"} {
		for _, network := range []C.NetWork{C.TCP, C.UDP} {
			rule := applicationRule{Package: "org.example.first", Target: target, RejectMissing: true}
			applicationRoutes.Store(&resolvedApplicationRules{
				owners: map[uint32]applicationRule{10001: rule},
			})
			beforeMissing, beforeRejected := applicationMissing.Load(), applicationMissingRejected.Load()
			beforeUDP := applicationUDPRejected.Load()
			metadata := &C.Metadata{
				NetWork:    network,
				RawSrcAddr: &net.TCPAddr{IP: net.ParseIP("198.18.0.1"), Port: 40000},
				RawDstAddr: &net.TCPAddr{IP: net.ParseIP("198.18.0.23"), Port: 443},
			}
			expected := rule.Package
			if target == "REJECT" {
				expected = "cor-reject:" + rule.Package
			}
			if result, err := applicationPackage(metadata); err != nil || result != expected {
				t.Fatalf("built-in %s selection failed: %q %v", target, result, err)
			}
			if applicationMissing.Load() != beforeMissing || applicationMissingRejected.Load() != beforeRejected {
				t.Fatalf("built-in %s was reported as missing", target)
			}
			if applicationUDPRejected.Load() != beforeUDP {
				t.Fatalf("built-in %s was reported as lacking UDP support", target)
			}
		}
	}
}

func TestOnlyTheInternalSelectorsEmptyFallbackCountsAsMissing(t *testing.T) {
	fallback := &fixtureApplicationProxy{name: "REJECT"}
	selected := &fixtureApplicationProxy{name: "REJECT"}
	selector := &fixtureApplicationSelector{proxies: []C.Proxy{fallback}, fallback: fallback}
	proxy := &fixtureApplicationProxy{name: "cor-application-example", adapter: selector}
	if !applicationEmptyTarget(proxy) {
		t.Fatal("empty provider selector was not detected")
	}
	selector.proxies = []C.Proxy{selected}
	if applicationEmptyTarget(proxy) {
		t.Fatal("a real target was confused with the empty fallback")
	}
	selector.proxies = []C.Proxy{fallback}
	proxy.name = "user-group"
	if applicationEmptyTarget(proxy) {
		t.Fatal("a user group was treated as the generated application selector")
	}
}
