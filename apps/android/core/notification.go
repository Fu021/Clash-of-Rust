package mobile

import (
	"encoding/json"

	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/tunnel"
	"github.com/metacubex/mihomo/tunnel/statistic"
)

// NotificationStatus reads only counters and the default selected chain. It
// works without an Activity and never allocates a full connection/proxy report.
func NotificationStatus() string {
	if !lifecycle.TryLock() {
		return "" // A reload is in progress; never delay service stop/cleanup.
	}
	defer lifecycle.Unlock()
	if !started {
		return ""
	}
	mode := tunnel.Mode().String()
	name := ""
	switch mode {
	case "global":
		name = "GLOBAL"
	case "direct":
		name = "DIRECT"
	default:
		for _, rule := range tunnel.Rules() {
			if rule.RuleType() == C.MATCH {
				name = rule.Adapter()
				break
			}
		}
	}
	proxies := tunnel.Proxies()
	var visited [16]string
selected:
	for depth := 0; depth < 16 && name != ""; depth++ {
		for _, previous := range visited[:depth] {
			if previous == name {
				name = ""
				break selected
			}
		}
		visited[depth] = name
		proxy, exists := proxies[name]
		if !exists {
			if depth == 0 {
				name = ""
			}
			break // A provider leaf may be absent from the global proxy map.
		}
		group, ok := proxy.Adapter().(interface{ Now() string })
		if !ok {
			break
		}
		name = group.Now()
		if depth == 15 {
			name = "" // Depth-limited chains are not confirmed leaves.
		}
	}
	up, down := statistic.DefaultManager.Now()
	data, _ := json.Marshal(struct {
		Mode string `json:"mode"`
		Node string `json:"node"`
		Up   int64  `json:"up"`
		Down int64  `json:"down"`
	}{mode, name, up, down})
	return string(data)
}
