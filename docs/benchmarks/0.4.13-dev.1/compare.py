"""Generate a comparison from two downloaded Memory Benchmark artifacts."""
import argparse
import json
from pathlib import Path
import statistics


def records(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def api_rows(root):
    result = {}
    for row in records(root/'api.jsonl'):
        if row.get('type') == 'api':
            key = row['workload'], row['mode'], row['refreshes']
            result.setdefault(key, []).append(row)
    return result


def median(rows, field):
    return statistics.median(row[field] for row in rows)


def peak(root, scene, role, field):
    values = [row[role][field] for row in records(root/(scene+'.jsonl'))
              if 'elapsed_ms' in row and row[role][field] is not None]
    return max(values) if values else None


def mib(value):
    return '不可用' if value is None else f'{value/1048576:.2f}'


def reduction(before, after):
    return '不可用' if not before or after is None else f'{(before-after)/before*100:.1f}%'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    before, after = (json.loads((root/'metadata.json').read_text()) for root in (args.before, args.after))
    for key in ('profile', 'rows', 'repetitions', 'gui_sample_seconds', 'interval_ms', 'rustc', 'cpu_count'):
        if before[key] != after[key]:
            raise ValueError('Non-comparable benchmark parameter: '+key)
    if (args.before/'gui-fixture.yaml').read_bytes() != (args.after/'gui-fixture.yaml').read_bytes():
        raise ValueError('GUI workloads differ')
    def resource_hashes(metadata):
        resources = metadata['resources']
        return (resources['core.json']['exe_sha256'],
                sorted((file['name'], file['sha256'], file['size'])
                       for file in resources['geodata.json']['files']))
    if resource_hashes(before) != resource_hashes(after):
        raise ValueError('Bundled core or Geo resources differ')
    lines = ['# 0.4.13-dev.1 内存对照', '',
             f"修改前 `{before['version']}` / `{before['source_commit']}`；修改后 `{after['version']}` / `{after['source_commit']}`。", '',
             f"两次均使用 GitHub Actions Ubuntu 22.04 x64、{before['profile']} 构建、相同 Rust 工具链和官方内核/Geo 资源。API 每项 {before['rows']:,} 条、{before['repetitions']} 次独立进程；数值取中位数。不同运行器的内核和调度可能存在差异，耗时仅作辅助参考。", '',
             '## API 堆内存', '', '单位 MiB，扣除输入 JSON 和运行时基线。连续刷新保留上一份快照直到新快照解析完成，模拟真实刷新峰值。', '',
             '| 数据 | 解析 | 刷新次数 | 峰值前 | 峰值后 | 驻留前 | 驻留后 | 驻留降低 | 耗时前/后 ms |',
             '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    old, new = api_rows(args.before), api_rows(args.after)
    if old.keys() != new.keys() or len(old) != 12:
        raise ValueError('API cases are incomplete')
    summary = {'before': before, 'after': after, 'api': [], 'gui': []}
    for key in sorted(old):
        if len(old[key]) != before['repetitions'] or len(new[key]) != after['repetitions']:
            raise ValueError('API repetitions are incomplete')
        values = {field: [median(old[key], field), median(new[key], field)]
                  for field in ('peak_extra_heap_bytes', 'retained_extra_heap_bytes', 'elapsed_ms')}
        summary['api'].append({'workload': key[0], 'mode': key[1], 'refreshes': key[2], **values})
        p, r, t = (values[field] for field in ('peak_extra_heap_bytes', 'retained_extra_heap_bytes', 'elapsed_ms'))
        lines.append(f'| {key[0]} | {key[1]} | {key[2]} | {mib(p[0])} | {mib(p[1])} | {mib(r[0])} | {mib(r[1])} | {reduction(*r)} | {t[0]:.1f}/{t[1]:.1f} |')
    lines += ['', '## GUI 与内核进程', '',
              '固定输入为 2,000 节点、40 个组各包含全部节点、20,000 条规则。关闭系统代理和定时测速，无业务流量。每场景采样 10 秒，间隔 250 ms；以下为观测峰值 MiB。搜索 `node-` 匹配所有节点，截图随原始结果保存。', '',
              '| 场景 | GUI RSS 前 | GUI RSS 后 | GUI RSS 降低 | 内核 RSS 前 | 内核 RSS 后 | 合计 PSS 前 | 合计 PSS 后 |',
              '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    for scene in ('home', 'proxies-collapsed', 'proxies-search', 'home-after-proxies'):
        values = {(role+'_'+field): [peak(root, scene, role, field) for root in (args.before, args.after)]
                  for role, field in (('gui', 'rss_bytes'), ('core', 'rss_bytes'), ('total', 'pss_bytes'))}
        summary['gui'].append({'scene': scene, **values})
        gui, core, pss = (values[key] for key in ('gui_rss_bytes', 'core_rss_bytes', 'total_pss_bytes'))
        lines.append(f'| {scene} | {mib(gui[0])} | {mib(gui[1])} | {reduction(*gui)} | {mib(core[0])} | {mib(core[1])} | {mib(pss[0])} | {mib(pss[1])} |')
    lines += ['', 'RSS 包含共享页，跨进程相加可能重复计数；PSS 分摊共享页。API 堆数值和进程内存不能相加。内核是独立 Go 程序，其波动不能直接归功于 Rust 优化。页面退出后分配器可能保留已释放内存供复用，RSS 不一定马上回落。', '',
              '## 修复范围', '',
              '| 此前发现的问题 | 已实施方案 |', '| --- | --- |',
              '| 超大本地订阅先读入再拒绝 | 文件大小预检和有界读取，10 MiB 上限 |',
              '| 端口保存覆盖排队期间的新设置 | 动作只携带端口，执行时合并最新设置 |',
              '| 同订阅 ID 更新后旧检测结果仍生效 | 取消句柄和任务代数，配置变化拒绝旧结果 |',
              '| 配置及模式提交失败导致磁盘和运行状态不一致 | 磁盘事务、双重恢复、启动时恢复中断事务；覆盖导入、更新、激活、删除和提权设置 |',
              '| API 快照重复文本、冗余字段及历史数组 | 每次响应内共享文本、紧凑字段、只保留最后延迟、收紧列表容量 |',
              '| 搜索同时创建大量组和节点 widget | 每页四组、每组六十节点的全局上限 |',
              '| 平台检测常量及响应临时分配 | 静态借用、UTF-8 缓冲复用、借用 JSON 字段、字符切片迭代、丢弃正文不缓存 |',
              '| 每个检测重复创建两套客户端 | 并发复用，Cookie 独立，TLS 1.3 按需，弱缓存释放 |', '',
              '## 原始记录与复现', '',
              '云端原始数据见 [cloud-before](cloud-before/README.md) 与 [cloud-after](cloud-after/README.md)。本目录 `before-*` 保留实施修复前取得的本地基线，未参与本报告的云端数值对照。', '',
              '统一基准为 `examples/memory_benchmark.rs`；手动 Memory Benchmark 工作流各运行一次即可取得相同格式。生成此报告：', '',
              '```bash', 'python3 docs/benchmarks/0.4.13-dev.1/compare.py docs/benchmarks/0.4.13-dev.1/cloud-before docs/benchmarks/0.4.13-dev.1/cloud-after --report /tmp/memory-comparison.md', '```', '']
    with args.report.open('x') as report:
        report.write('\n'.join(lines))
    with args.report.with_suffix('.json').open('x') as report:
        json.dump(summary, report, ensure_ascii=False, indent=2)
        report.write('\n')


if __name__ == '__main__':
    main()
