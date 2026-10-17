using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

// Persistent app-specific ALE blocks, including loopback. This does not change
// Windows firewall profiles, DNS, adapters, or the existing Mihomo provider.
public static class CorDockerEgressGuard
{
    static readonly Guid Provider = new Guid("15926cc4-e22c-4d75-9871-12c7a55818d0");
    static readonly Guid Sublayer = new Guid("c19db3a8-568d-46bb-96bb-bb54f848fc21");
    static readonly Guid[] Layers = {
        new Guid("c38d57d1-05a7-4c33-904f-7fbceee60e82"),
        new Guid("4a72393b-319f-44bc-84c3-ba54dcb3b6b4") };
    static readonly Guid App = new Guid("d78e1e87-8644-4ea5-9437-d809ecefc971");
    static readonly byte[] Marker = Encoding.ASCII.GetBytes("ClashOfRust.WslDockerGuard/v1");
    const uint MissingFilter = 0x80320003, MissingProvider = 0x80320005, MissingSublayer = 0x80320007;
    const uint Persistent = 1, Indexed = 0x40, Block = 0x1001;

    [StructLayout(LayoutKind.Sequential)] struct Display { public IntPtr name, description; }
    [StructLayout(LayoutKind.Sequential)] struct Blob { public uint size; public IntPtr data; }
    [StructLayout(LayoutKind.Explicit, Size=16)] struct Value {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public ulong scalar;
        [FieldOffset(8)] public IntPtr pointer;
    }
    [StructLayout(LayoutKind.Sequential)] struct Condition { public Guid key; public uint match; public Value value; }
    [StructLayout(LayoutKind.Sequential)] struct Action { public uint type; public Guid key; }
    [StructLayout(LayoutKind.Explicit, Size=16)] struct Context { [FieldOffset(0)] public ulong raw; }
    [StructLayout(LayoutKind.Sequential)] struct Filter {
        public Guid key; public Display display; public uint flags; public IntPtr provider;
        public Blob data; public Guid layer, sublayer; public Value weight;
        public uint count; public IntPtr conditions; public Action action; public Context context;
        public IntPtr reserved; public ulong id; public Value effectiveWeight;
    }
    [StructLayout(LayoutKind.Sequential)] struct ProviderInfo {
        public Guid key; public Display display; public uint flags; public Blob data; public IntPtr serviceName;
    }
    [StructLayout(LayoutKind.Sequential)] struct SublayerInfo {
        public Guid key; public Display display; public uint flags; public IntPtr provider;
        public Blob data; public ushort weight;
    }
    [DllImport("fwpuclnt.dll", CharSet=CharSet.Unicode)] static extern uint FwpmEngineOpen0(string server, uint auth, IntPtr identity, IntPtr session, out IntPtr engine);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmEngineClose0(IntPtr engine);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmTransactionBegin0(IntPtr engine, uint flags);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmTransactionCommit0(IntPtr engine);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmTransactionAbort0(IntPtr engine);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmProviderAdd0(IntPtr engine, ref ProviderInfo provider, IntPtr security);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmProviderGetByKey0(IntPtr engine, ref Guid key, out IntPtr result);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmSubLayerAdd0(IntPtr engine, ref SublayerInfo layer, IntPtr security);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmSubLayerGetByKey0(IntPtr engine, ref Guid key, out IntPtr result);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmFilterAdd0(IntPtr engine, ref Filter filter, IntPtr security, out ulong id);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmFilterGetByKey0(IntPtr engine, ref Guid key, out IntPtr result);
    [DllImport("fwpuclnt.dll")] static extern uint FwpmFilterDeleteByKey0(IntPtr engine, ref Guid key);
    [DllImport("fwpuclnt.dll", CharSet=CharSet.Unicode)] static extern uint FwpmGetAppIdFromFileName0(string path, out IntPtr blob);
    [DllImport("fwpuclnt.dll")] static extern void FwpmFreeMemory0(ref IntPtr memory);

    sealed class Memory : IDisposable {
        readonly List<IntPtr> pointers = new List<IntPtr>();
        public IntPtr Structure<T>(T value) {
            IntPtr p=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(T))); pointers.Add(p);
            Marshal.StructureToPtr(value,p,false); return p;
        }
        public Display Label(string name) {
            IntPtr p=Marshal.StringToHGlobalUni(name); pointers.Add(p); return new Display {name=p};
        }
        public Blob Bytes(byte[] bytes) {
            IntPtr p=Marshal.AllocHGlobal(bytes.Length); pointers.Add(p);
            Marshal.Copy(bytes,0,p,bytes.Length); return new Blob {size=(uint)bytes.Length,data=p};
        }
        public void Dispose() { foreach(IntPtr p in pointers) Marshal.FreeHGlobal(p); }
    }
    static void Check(uint code) {
        if(code!=0) throw new InvalidOperationException("WSL Docker WFP operation failed: 0x"+code.ToString("x8"));
    }
    public static void ValidateLayout() {
        if(IntPtr.Size!=8 || Marshal.SizeOf(typeof(Filter))!=200 || Marshal.SizeOf(typeof(Condition))!=40
            || Marshal.SizeOf(typeof(SublayerInfo))!=72 || Marshal.SizeOf(typeof(ProviderInfo))!=64
            || Marshal.OffsetOf(typeof(Filter),"conditions").ToInt32()!=120
            || Marshal.OffsetOf(typeof(Filter),"context").ToInt32()!=152)
            throw new PlatformNotSupportedException("Unexpected WFP ABI; no engine opened.");
    }
    static string Normalize(string path) {
        ValidateLayout();
        if(String.IsNullOrWhiteSpace(path) || !Path.IsPathRooted(path)) throw new ArgumentException("Local executable required.");
        string p=Path.GetFullPath(path);
        if(p.StartsWith(@"\\",StringComparison.Ordinal) || p.IndexOf(':',2)>=0 || !File.Exists(p)
            || !String.Equals(Path.GetExtension(p),".exe",StringComparison.OrdinalIgnoreCase))
            throw new ArgumentException("Existing local executable required.");
        return p;
    }
    static Guid Key(string path,int index) {
        using(SHA256 sha=SHA256.Create()) {
            byte[] bytes=sha.ComputeHash(Encoding.UTF8.GetBytes("CorDocker/v1:"+path.ToLowerInvariant()+":"+index));
            byte[] key=new byte[16]; Array.Copy(bytes,key,16); return new Guid(key);
        }
    }
    static bool Same(Blob blob,byte[] bytes) {
        if(blob.size!=bytes.Length || blob.data==IntPtr.Zero) return false;
        byte[] actual=new byte[bytes.Length]; Marshal.Copy(blob.data,actual,0,actual.Length);
        for(int i=0;i<actual.Length;i++) if(actual[i]!=bytes[i]) return false;
        return true;
    }
    static byte[] BlobBytes(IntPtr pointer) {
        Blob b=(Blob)Marshal.PtrToStructure(pointer,typeof(Blob));
        if(b.size>1048576 || b.data==IntPtr.Zero) throw new InvalidOperationException("Invalid WFP blob.");
        byte[] bytes=new byte[b.size]; Marshal.Copy(b.data,bytes,0,bytes.Length); return bytes;
    }
    static bool GuidAt(IntPtr p,Guid expected) { return p!=IntPtr.Zero && (Guid)Marshal.PtrToStructure(p,typeof(Guid))==expected; }
    static string Name(int i) { return "Clash of Rust: Docker egress blocked IPv"+(i==0?"4":"6"); }
    static bool Metadata(IntPtr engine,bool create,Memory memory) {
        IntPtr pp=IntPtr.Zero,sp=IntPtr.Zero; Guid pk=Provider,sk=Sublayer;
        uint pc=FwpmProviderGetByKey0(engine,ref pk,out pp),sc=FwpmSubLayerGetByKey0(engine,ref sk,out sp);
        try {
            if(pc==MissingProvider && sc==MissingSublayer) {
                if(!create) return false;
                ProviderInfo provider=new ProviderInfo {key=Provider,flags=Persistent,display=memory.Label("Clash of Rust WSL Docker guard"),data=memory.Bytes(Marker)};
                Check(FwpmProviderAdd0(engine,ref provider,IntPtr.Zero));
                SublayerInfo layer=new SublayerInfo {key=Sublayer,flags=Persistent,provider=memory.Structure(Provider),data=memory.Bytes(Marker),weight=0xf100,display=memory.Label("WSL Docker complete egress deny")};
                Check(FwpmSubLayerAdd0(engine,ref layer,IntPtr.Zero)); return true;
            }
            Check(pc); Check(sc);
            ProviderInfo p=(ProviderInfo)Marshal.PtrToStructure(pp,typeof(ProviderInfo));
            SublayerInfo s=(SublayerInfo)Marshal.PtrToStructure(sp,typeof(SublayerInfo));
            if(p.key!=Provider || p.flags!=Persistent || p.serviceName!=IntPtr.Zero || !Same(p.data,Marker)
                || s.key!=Sublayer || s.flags!=Persistent || !GuidAt(s.provider,Provider) || !Same(s.data,Marker)
                || s.weight!=0xf100) throw new InvalidOperationException("Foreign WFP metadata; preserved.");
            return true;
        } finally { if(pp!=IntPtr.Zero) FwpmFreeMemory0(ref pp); if(sp!=IntPtr.Zero) FwpmFreeMemory0(ref sp); }
    }
    static bool Existing(IntPtr engine,string path,int index,IntPtr appId) {
        Guid key=Key(path,index); IntPtr pointer; uint code=FwpmFilterGetByKey0(engine,ref key,out pointer);
        if(code==MissingFilter) return false; Check(code);
        try {
            Filter f=(Filter)Marshal.PtrToStructure(pointer,typeof(Filter));
            if(f.key!=key || (f.flags&Persistent)==0 || (f.flags&~(Persistent|Indexed))!=0
                || !GuidAt(f.provider,Provider) || f.layer!=Layers[index] || f.sublayer!=Sublayer
                || f.action.type!=Block || f.action.key!=Guid.Empty || f.count!=1 || f.conditions==IntPtr.Zero
                || f.weight.type!=1 || (byte)f.weight.scalar!=15 || f.context.raw!=0 || !Same(f.data,Marker)
                || Marshal.PtrToStringUni(f.display.name)!=Name(index)) throw new InvalidOperationException("Foreign WFP filter; preserved.");
            Condition c=(Condition)Marshal.PtrToStructure(f.conditions,typeof(Condition));
            if(c.key!=App || c.match!=0 || c.value.type!=12 || c.value.pointer==IntPtr.Zero
                || !Same((Blob)Marshal.PtrToStructure(c.value.pointer,typeof(Blob)),BlobBytes(appId)))
                throw new InvalidOperationException("Foreign WFP condition; preserved.");
            return true;
        } finally { FwpmFreeMemory0(ref pointer); }
    }
    public static void Apply(string[] paths,bool remove) { Operate(paths,remove,false); }
    public static bool Verify(string[] paths) { return Operate(paths,false,true); }
    static bool Operate(string[] paths,bool remove,bool readOnly) {
        if(paths==null || paths.Length==0) throw new ArgumentException("Executable list required.");
        var names=new List<string>(); var ids=new List<IntPtr>(); var unique=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach(string input in paths) { string p=Normalize(input); if(!unique.Add(p)) throw new ArgumentException("Duplicate executable."); names.Add(p); }
        IntPtr engine=IntPtr.Zero; bool transaction=false;
        try {
            foreach(string p in names) { IntPtr id; Check(FwpmGetAppIdFromFileName0(p,out id)); ids.Add(id); }
            Check(FwpmEngineOpen0(null,10,IntPtr.Zero,IntPtr.Zero,out engine));
            Check(FwpmTransactionBegin0(engine,readOnly?1u:0u)); transaction=true;
            using(Memory memory=new Memory()) {
                bool present=Metadata(engine,!remove&&!readOnly,memory);
                bool all=present; bool[,] existing=new bool[names.Count,2];
                for(int p=0;p<names.Count;p++) for(int i=0;i<2;i++) { existing[p,i]=Existing(engine,names[p],i,ids[p]); all&=existing[p,i]; }
                if(!readOnly) for(int p=0;p<names.Count;p++) for(int i=0;i<2;i++) {
                    Guid key=Key(names[p],i);
                    if(existing[p,i]) Check(FwpmFilterDeleteByKey0(engine,ref key));
                    if(remove) continue;
                    Condition condition=new Condition {key=App,value=new Value {type=12,pointer=ids[p]}};
                    Filter f=new Filter {key=key,display=memory.Label(Name(i)),flags=Persistent,provider=memory.Structure(Provider),
                        data=memory.Bytes(Marker),layer=Layers[i],sublayer=Sublayer,weight=new Value {type=1,scalar=15},
                        count=1,conditions=memory.Structure(condition),action=new Action {type=Block}};
                    ulong id; Check(FwpmFilterAdd0(engine,ref f,IntPtr.Zero,out id));
                }
                Check(FwpmTransactionCommit0(engine)); transaction=false; return all;
            }
        } finally {
            if(transaction) FwpmTransactionAbort0(engine);
            if(engine!=IntPtr.Zero) FwpmEngineClose0(engine);
            foreach(IntPtr id in ids) { IntPtr pointer=id; FwpmFreeMemory0(ref pointer); }
        }
    }
}
