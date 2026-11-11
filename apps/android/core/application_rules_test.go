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
	fail        bool
}

func (owner *fixtureOwner) Owner(protocol int32, source string, sourcePort int32, destination string, destinationPort int32) (int32, error) {
	owner.source = source
	owner.destination = destination
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
	if result, err := applicationPackage(metadata); err != nil || result != "cor-reject:org.example.first" {
		t.Fatalf("missing-target rejection lost its independent matching identity: %q %v", result, err)
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
