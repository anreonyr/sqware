#!/usr/bin/env python3
"""Compare eb2a01d Vec maps with workspace maps in isolated release snapshots.
Run from any directory: python3 scripts/bench-map-index.py
Outputs build logs, guest logs, and ELFs in /tmp/sqware-map-bench.
Use --tables to compare kernel page-table counts against 2225340.
Requires cargo, nu, nm, qemu-system-riscv64. Does not edit kernel sources.
"""
import pathlib, subprocess, tarfile, io, os, json, argparse
root=pathlib.Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--tables', action='store_true', help='Compare kernel page-table counts in debug builds')
options=parser.parse_args()
out=pathlib.Path('/tmp/sqware-page-tables' if options.tables else '/tmp/sqware-map-bench')
out.mkdir(exist_ok=True)
source=r'''use core::fmt::Write;
use core::hint::black_box;
use core::sync::atomic::{compiler_fence, Ordering};
use crate::memory::PAGE_SIZE as P;
use crate::memory::manager::{addr::VirtAddr, entry::PteFlags};
use super::{SpaceBuilder, Pending};
use super::map::Map;
use super::salvage::Salvage;
const BASE: usize = 0x4000_0000;
fn tick() -> usize { compiler_fence(Ordering::SeqCst); let t = riscv::register::time::read(); compiler_fence(Ordering::SeqCst); t }
pub fn run() {
    for n in [8usize, 32, 128, 512] {
        for round in 0..5 {
            let space = SpaceBuilder::user().build().unwrap();
            let flags = space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
            space.with(|inner| {
                for i in 0..n { inner.map(VirtAddr::from_raw(BASE + i*16*P), 8*P, flags, Some(Pending::Lazy)).unwrap(); }
                if round == 0 { writeln!(crate::console::Sink, "MEM,{},{},{},{},{}", n, core::mem::size_of::<Map>(), core::mem::size_of_val(&inner.maps), inner.maps.len(), CAPACITY_BYTES); }
                for kind in 0..4 {
                    let mut keys = [0usize; 1024];
                    for (i, key) in keys.iter_mut().enumerate() { *key = BASE + ((i*37+11)%n)*16*P + if kind%2 == 0 { 3*P } else { 12*P }; }
                    let start = tick();
                    let mut hits = 0usize;
                    for i in 0..32768 {
                        let va = VirtAddr::from_raw(black_box(keys[i%1024]));
                        let hit = if kind < 2 { inner.resolve_ref(va).is_some() } else { inner.overlaps(va, 2*P) };
                        hits += black_box(hit) as usize;
                    }
                    let elapsed = tick()-start;
                    assert_eq!(hits, if kind%2 == 0 {32768} else {0});
                    writeln!(crate::console::Sink, "BENCH,{},{},{},{},{}", n, round, kind, elapsed, 32768);
                }
            });
            drop(space);
            for kind in 4..6 {
                let space = SpaceBuilder::user().build().unwrap();
                let flags = space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
                let mut salvage = Salvage::new();
                space.with(|inner| {
                    for i in 0..n { inner.map(VirtAddr::from_raw(BASE+i*16*P),8*P,flags,Some(Pending::Lazy)).unwrap(); }
                    let start = tick();
                    for i in 0..n {
                        let va = VirtAddr::from_raw(BASE+((i*37+11)%n)*16*P+3*P);
                        if kind == 4 { inner.protect(va,2*P,PteFlags::R,true).unwrap(); }
                        else { inner.unmap(va,2*P,&mut salvage).unwrap(); }
                    }
                    let elapsed = tick()-start;
                    writeln!(crate::console::Sink, "BENCH,{},{},{},{},{}", n, round, kind, elapsed, n);
                    for i in 0..n {
                        let va = VirtAddr::from_raw(BASE+i*16*P+3*P);
                        let map = inner.resolve_ref(va);
                        if kind == 4 { assert!(!map.unwrap().flags.contains(PteFlags::W)); }
                        else { assert!(map.is_none()); }
                    }
                });
                salvage.reclaim(&space).unwrap();
            }
        }
    }
}
'''
if options.tables:
    source=r"""use core::fmt::Write;
pub fn run() {
    let space = &crate::work::unit::team::kernel().unwrap().space;
    writeln!(crate::console::Sink, "TABLES,{}", space.table_count()).unwrap();
}
"""
baseline='2225340' if options.tables else 'eb2a01d'
archive=subprocess.check_output(['git','archive',baseline],cwd=root)
changes=['kernel/src/memory/manager/table.rs', 'kernel/src/memory/manager/mod.rs', 'kernel/src/memory/manager/asid.rs', 'kernel/src/work/unit/space/inner.rs', 'kernel/src/work/unit/space/map.rs', 'kernel/src/work/unit/space/mod.rs', 'kernel/src/work/unit/space/outer.rs', 'kernel/src/work/unit/space/salvage.rs', 'kernel/src/work/unit/space/window/stack.rs', 'kernel/src/work/unit/space/index.rs']
for variant in (['before','after'] if options.tables else ['vec','tree']):
    dst=out/variant; dst.mkdir(exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(dst,filter='data')
    if variant in ['tree','after']:
        for name in changes: (dst/name).write_bytes((root/name).read_bytes())
    folder=dst/'kernel/src/work/unit/space'
    (folder/'bench.rs').write_text(source.replace('CAPACITY_BYTES','inner.maps.capacity()*core::mem::size_of::<alloc::boxed::Box<Map>>()' if variant=='vec' else '0usize'))
    f=folder/'mod.rs'; f.write_text('pub(crate) mod bench;\n'+f.read_text())
    f=dst/'kernel/src/lib.rs'; f.write_text(f.read_text()+'\npub use work::unit::space::bench::run as map_bench;\n')
    f=dst/'kernel/tests/embedded.rs'; text=f.read_text(); text=text[:text.index('    // 用例：')]+'''    #[test]
    fn memory_index_benchmark() { kernel::map_bench(); }
}
'''; f.write_text(text)
    env=dict(os.environ,CARGO_TARGET_DIR=str(out/'target'))
    p=subprocess.run(['cargo','test','-p','kernel','--test','embedded']+([] if options.tables else ['--release'])+['--no-run','--message-format=json'],cwd=dst,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    (out/(variant+'-build.log')).write_text(p.stderr)
    if p.returncode:
        print(p.stderr)
        for line in p.stdout.splitlines():
            d=json.loads(line)
            if d.get("reason")=="compiler-message": print(d["message"]["rendered"])
        raise SystemExit(p.returncode)
    exe=None
    for line in p.stdout.splitlines():
        data=json.loads(line)
        if data.get('executable') and data.get('target', {}).get('name') == 'embedded':
            exe=data['executable']
    assert exe
    (out/(variant+'.elf')).write_bytes(pathlib.Path(exe).read_bytes())
    env=dict(os.environ,QEMU_ICOUNT='0,sleep=off',QEMU_SMP='1')
    args=subprocess.check_output(['nu','scripts/qemu-args.nu','--board-only'],cwd=dst,env=env,text=True).splitlines()
    symbols=subprocess.check_output(['nm',str(out/(variant+'.elf'))],text=True).splitlines()
    address=int(next(l.split()[0] for l in symbols if '___memory_index_benchmark_entrypoint' in l),16)
    args+=['-kernel',str(out/(variant+'.elf')),'-semihosting-config',f'enable=on,target=native,arg=run_addr,arg={address}']
    p=subprocess.run(['qemu-system-riscv64']+args,cwd=dst,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=120)
    (out/(variant+'.log')).write_text(p.stdout)
    print(variant, 'exit',p.returncode,flush=True)
    print('\n'.join(l for l in p.stdout.splitlines() if 'MEM,' in l or 'BENCH,' in l or 'TABLES,' in l),flush=True)
    if p.returncode: print(p.stdout); raise SystemExit(p.returncode)
