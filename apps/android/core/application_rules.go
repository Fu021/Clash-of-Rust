package mobile

import (
	"encoding/json"
	"errors"
	"net/netip"
	"sync/atomic"
	"time"

	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/log"
	"github.com/metacubex/mihomo/tunnel"
	"go.yaml.in/yaml/v3"
)

type ConnectionOwner interface {
	Owner(protocol int32, source string, sourcePort int32, destination string, destinationPort int32) (int32, error)
	PackageUID(packageName string) (int32, error)
}

type ownerHolder struct{ value ConnectionOwner }
type applicationRule struct {
	Package       string `yaml:"package"`
	Target        string `yaml:"target"`
	RejectMissing bool   `yaml:"reject_missing"`
}
type applicationRules struct {
	Schema int               `yaml:"schema"`
	Routes []applicationRule `yaml:"routes"`
}
type resolvedApplicationRules struct {
	owners  map[uint32]applicationRule
	proxies map[string]C.Proxy
}

var (
	connectionOwner        atomic.Pointer[ownerHolder]
	applicationRoutes      atomic.Pointer[resolvedApplicationRules]
	ownerQueries           atomic.Uint64
	ownerFailures          atomic.Uint64
	applicationMatches     atomic.Uint64
	applicationMissing     atomic.Uint64
	applicationUDPRejected atomic.Uint64
	lastOwnerWarning       atomic.Int64
)

func SetConnectionOwner(value ConnectionOwner) {
	if value == nil {
		connectionOwner.Store(nil)
	} else {
		connectionOwner.Store(&ownerHolder{value})
	}
}

func parseApplicationRules(raw string) (*resolvedApplicationRules, error) {
	var config struct {
		Rules applicationRules `yaml:"cor-application-rules"`
	}
	if err := yaml.Unmarshal([]byte(raw), &config); err != nil {
		return nil, err
	}
	resolved := &resolvedApplicationRules{owners: make(map[uint32]applicationRule)}
	if len(config.Rules.Routes) == 0 {
		return resolved, nil
	}
	if config.Rules.Schema != 1 || len(config.Rules.Routes) > 256 {
		return nil, errors.New("invalid application routing schema or limit")
	}
	owner := connectionOwner.Load()
	if owner == nil {
		return nil, errors.New("application routing is unavailable on this Android version")
	}
	for _, rule := range config.Rules.Routes {
		uid, err := owner.value.PackageUID(rule.Package)
		if err != nil {
			return nil, errors.New("could not resolve application identity")
		}
		if uid <= 0 {
			continue
		}
		if previous, exists := resolved.owners[uint32(uid)]; exists &&
			(previous.Target != rule.Target || previous.RejectMissing != rule.RejectMissing) {
			return nil, errors.New("applications sharing a UID must use the same outbound")
		}
		resolved.owners[uint32(uid)] = rule
	}
	return resolved, nil
}

func ApplicationRoutingDiagnostics() string {
	value, _ := json.Marshal(map[string]any{
		"supported":       connectionOwner.Load() != nil,
		"owner_queries":   ownerQueries.Load(),
		"owner_failures":  ownerFailures.Load(),
		"matches":         applicationMatches.Load(),
		"missing_targets": applicationMissing.Load(),
		"udp_rejected":    applicationUDPRejected.Load(),
	})
	return string(value)
}

func applicationTarget(rule applicationRule, available, pass, udp, supportsUDP bool) string {
	if rule.Target == "" || !available || pass {
		if rule.RejectMissing {
			return "REJECT"
		}
		return ""
	}
	if udp && !supportsUDP {
		return "REJECT"
	}
	return rule.Target
}

func applicationPackage(metadata *C.Metadata) (string, error) {
	rules := applicationRoutes.Load()
	owner := connectionOwner.Load()
	if rules == nil || len(rules.owners) == 0 || owner == nil || tunnel.Mode() != tunnel.Rule {
		return "", nil
	}
	protocol := int32(6)
	if metadata.NetWork == C.UDP {
		protocol = 17
	}
	ownerQueries.Add(1)
	var uid int32
	var err error
	if metadata.RawSrcAddr == nil || metadata.RawDstAddr == nil {
		err = errors.New("original connection tuple is unavailable")
	} else {
		source, sourceErr := netip.ParseAddrPort(metadata.RawSrcAddr.String())
		destination, destinationErr := netip.ParseAddrPort(metadata.RawDstAddr.String())
		if sourceErr != nil || destinationErr != nil {
			err = errors.New("invalid original connection tuple")
		} else {
			uid, err = owner.value.Owner(protocol, source.Addr().String(), int32(source.Port()),
				destination.Addr().String(), int32(destination.Port()))
		}
	}
	if err != nil || uid <= 0 {
		ownerFailures.Add(1)
		now := time.Now().Unix()
		last := lastOwnerWarning.Load()
		if now-last >= 60 && lastOwnerWarning.CompareAndSwap(last, now) {
			log.Warnln("[ApplicationRouting] APP_OWNER_LOOKUP_FAILED; connection follows configuration")
		}
		return "", errors.New("APP_OWNER_LOOKUP_FAILED")
	}
	metadata.Uid = uint32(uid)
	rule, exists := rules.owners[uint32(uid)]
	if !exists {
		return "", nil
	}
	currentUID, identityErr := owner.value.PackageUID(rule.Package)
	if identityErr != nil || currentUID != uid {
		ownerFailures.Add(1)
		return "", errors.New("APP_OWNER_IDENTITY_CHANGED")
	}
	proxy, available := rules.proxies[rule.Target]
	pass, supportsUDP := false, false
	if available {
		supportsUDP = proxy.SupportUDP()
		visited := 0
		for adapter := C.ProxyAdapter(proxy); adapter != nil; adapter = adapter.Unwrap(metadata, false) {
			if adapter.Type() == C.Pass {
				pass = true
				break
			}
			visited++
			if visited > 32 {
				available = false
				break
			}
		}
	}
	target := applicationTarget(rule, available, pass, metadata.NetWork == C.UDP, supportsUDP)
	if !available || pass || rule.Target == "" {
		applicationMissing.Add(1)
	}
	if metadata.NetWork == C.UDP && available && !pass && !supportsUDP {
		applicationUDPRejected.Add(1)
	}
	if target != "" {
		applicationMatches.Add(1)
		if target == "REJECT" {
			return "cor-reject:" + rule.Package, nil
		}
		return rule.Package, nil
	}
	return "", nil
}
