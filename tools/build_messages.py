#!/usr/bin/env python3
"""Selectively extract and merge native Goods text. No game launch or tests."""
from __future__ import annotations
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import uuid
import urllib.parse
import urllib.request

from environment_probe import PROJECT, WORKSPACE, locate, pe_version, steam_root
from params_tooling import digest, extract_archive

SMITHBOX_REV = "6625a5857120b5dfe3b342f1dce5d953754d420a"
SOURCE_REV = "ee1dd61958f60bdc51ce3da548e9a90a8ab39905"
DEFAULT_DOTNET = Path(os.environ.get('ASCENSION_DOTNET', 'dotnet'))
DEFAULT_LIBRARY = Path(os.environ.get('ASCENSION_SOULSFORMATS', 'SoulsFormats.dll'))
WINDOWS_RUNTIME = '9.0.22'
WINDOWS_RUNTIME_URL = f'https://builds.dotnet.microsoft.com/dotnet/Runtime/{WINDOWS_RUNTIME}/dotnet-runtime-{WINDOWS_RUNTIME}-win-x64.zip'
WINDOWS_RUNTIME_SHA512 = 'ebe32c4c7b69a9d3b9b822678a46846f4dc075b1b82f1868e17d2f61ad089d75440da97471fff554dd953b34cc134ea49ac86a18e14265df8b686e23565e1dcb'
HELPER_SOURCE = r'''
using System.Numerics;
using System.Formats.Asn1;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using SoulsFormats;

Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
if (Environment.GetEnvironmentVariable("ASCENSION_OODLE") is string oodle)
    Oodle.Oodle6Ptr=System.Runtime.InteropServices.NativeLibrary.Load(oodle);
static string Sha(byte[] data) => Convert.ToHexString(SHA256.HashData(data)).ToLowerInvariant();
static ulong PathHash(string path) {
    var normalized = "/" + path.Replace('\\','/').Trim('/').ToLowerInvariant();
    ulong hash = 0;
    foreach (char c in normalized) hash = unchecked(hash * 0x85UL + c);
    return hash;
}
static byte[] Header(string path, string pem) {
    byte[] source = File.ReadAllBytes(path);
    if (Encoding.ASCII.GetString(source, 0, 4) == "BHD5") return source;
    string encoded=string.Concat(File.ReadAllLines(pem).Where(line=>!line.StartsWith("-----")));
    var reader=new AsnReader(Convert.FromBase64String(encoded),AsnEncodingRules.DER);
    var sequence=reader.ReadSequence();var modulus=sequence.ReadInteger();var exponent=sequence.ReadInteger();
    sequence.ThrowIfNotEmpty();reader.ThrowIfNotEmpty();
    int input = modulus.GetByteCount(isUnsigned:true), output = input - 1;
    if (source.Length % input != 0) throw new InvalidDataException("Incomplete RSA archive header");
    using var result = new MemoryStream();
    byte[] zeros = new byte[output];
    for (int offset=0; offset<source.Length; offset+=input) {
        var value = new BigInteger(source.AsSpan(offset,input), true, true);
        byte[] clear = BigInteger.ModPow(value, exponent, modulus).ToByteArray(true,true);
        if (clear.Length > output) throw new InvalidDataException("Invalid RSA output block");
        result.Write(zeros, 0, output-clear.Length); result.Write(clear);
    }
    return result.ToArray();
}
var options = new JsonSerializerOptions { WriteIndented=true };
try {
if (args[0] == "extract") {
    string game=args[1], keys=args[2], destination=args[3];
    var languages=args[4].Split(','); string container=args[5];
    var wanted=languages.ToDictionary(lang=>PathHash($"msg/{lang}/{container}"), lang=>$"msg/{lang}/{container}");
    var extracted=new List<object>(); var archives=new List<object>();
    foreach (string archive in new[]{"Data0","Data1","Data2","Data3","DLC"}) {
        if (wanted.Count==0) break;
        string bhd=Path.Combine(game,archive+".bhd"), bdt=Path.Combine(game,archive+".bdt"), pem=Path.Combine(keys,archive+".pem");
        if (!File.Exists(bhd) || !File.Exists(bdt) || !File.Exists(pem)) continue;
        byte[] clear=Header(bhd,pem);
        var index=BHD5.Read(clear,BHD5.Game.EldenRing);
        archives.Add(new {name=archive+".bhd",sha256=Sha(File.ReadAllBytes(bhd)),bdt_size=new FileInfo(bdt).Length});
        using var data=File.OpenRead(bdt);
        foreach (var entry in index.Buckets.SelectMany(bucket=>bucket)) {
            if (!wanted.TryGetValue(entry.FileNameHash,out var relative)) continue;
            if (entry.PaddedFileSize<=0 || entry.PaddedFileSize>64*1024*1024 || entry.FileOffset<0 || entry.FileOffset+entry.PaddedFileSize>data.Length)
                throw new InvalidDataException("Invalid selective message entry bounds");
            byte[] bytes=new byte[entry.PaddedFileSize];data.Position=entry.FileOffset;data.ReadExactly(bytes);
            entry.AESKey?.Decrypt(bytes);
            if (entry.UnpaddedFileSize>0 && entry.UnpaddedFileSize<=bytes.Length) Array.Resize(ref bytes,(int)entry.UnpaddedFileSize);
            var binder=BND4.Read(bytes);
            string output=Path.Combine(destination,relative.Replace('/',Path.DirectorySeparatorChar));
            Directory.CreateDirectory(Path.GetDirectoryName(output)!);File.WriteAllBytes(output,bytes);
            extracted.Add(new {path=relative,archive=archive+".bhd",source_sha256=Sha(bytes),size=bytes.Length,
                members=binder.Files.Select(file=>new {id=file.ID,name=file.Name}).ToArray()});
            wanted.Remove(entry.FileNameHash);
        }
    }
    var manifest=new {schema=1,archives,files=extracted,missing=wanted.Values.ToArray()};
    File.WriteAllText(Path.Combine(destination,"source-manifest.json"),JsonSerializer.Serialize(manifest,options));
    Console.WriteLine(JsonSerializer.Serialize(new {extracted=extracted.Count,missing=wanted.Count},options));
} else if (args[0] == "merge") {
    string source=args[1], destination=args[2];
    using var labels=JsonDocument.Parse(File.ReadAllText(args[3]));
    var roles=labels.RootElement.GetProperty("roles").EnumerateObject().ToDictionary(prop=>int.Parse(prop.Name),prop=>prop.Value.GetString()!);
    var additions=labels.RootElement.GetProperty("entries").EnumerateArray().ToArray();
    var binder=BND4.Read(source);var merged=new List<object>();
    foreach (var file in binder.Files) {
        if (!roles.TryGetValue(file.ID,out string? role)) continue;
        var fmg=FMG.Read(file.Bytes);int before=fmg.Entries.Count;
        foreach (var entry in additions) {
            int id=entry.GetProperty("id").GetInt32();
            if (fmg.Entries.Any(existing=>existing.ID==id)) throw new InvalidDataException($"Goods text ID collision {id} in FMG {file.ID}");
            fmg.Entries.Add(new FMG.Entry(id,entry.GetProperty(role).GetString()!));
        }
        fmg.Entries=fmg.Entries.OrderBy(entry=>entry.ID).ToList();file.Bytes=fmg.Write();
        merged.Add(new {id=file.ID,role,before,after=fmg.Entries.Count,added=additions.Length});
    }
    if (!merged.Any()) throw new InvalidDataException("No Goods text tables found");
    Directory.CreateDirectory(Path.GetDirectoryName(destination)!);binder.Write(destination);
    File.WriteAllText(destination+".merge.json",JsonSerializer.Serialize(new {tables=merged},options));
    Console.WriteLine(JsonSerializer.Serialize(new {tables=merged.Count,entries_per_table=additions.Length},options));
} else throw new ArgumentException("Expected extract or merge");
} catch (Exception error) { Console.Error.WriteLine(error); Environment.Exit(2); }
'''


def fetch(url, path):
    path = Path(path)
    if not path.is_file():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url, timeout=60) as source, path.open('xb') as output:
            shutil.copyfileobj(source, output)
    return path


def prepare(work, dotnet, library):
    work.mkdir(parents=True, exist_ok=True)
    if not dotnet.is_file() or not library.is_file():
        raise ValueError('Provide an installed .NET9 SDK and official SoulsFormatsNEXT assembly using --dotnet / --soulsformats')
    source = fetch(f'https://raw.githubusercontent.com/vawser/Smithbox/{SMITHBOX_REV}/src/Andre/Andre.Formats/Util/ArchiveKeys.cs', work/'ArchiveKeys.cs')
    section = source.read_text(encoding='utf-8-sig').split('EldenRingKeys =',1)[1].split('ArmoredCore6Keys =',1)[0]
    keys = work/'keys'; keys.mkdir(exist_ok=True)
    for match in re.finditer(r'\["([^"]+)"\]\s*=\s*@"(-----BEGIN RSA PUBLIC KEY-----.*?-----END RSA PUBLIC KEY-----)"',section,re.S):
        if match.group(1) in {'Data0','Data1','Data2','Data3','DLC'}:
            (keys/(match.group(1)+'.pem')).write_text(match.group(2))
    registry_path = fetch('https://raw.githubusercontent.com/vawser/Smithbox/'+SMITHBOX_REV+'/'+urllib.parse.quote('src/Smithbox.Data/Assets/FMG/ER/Fmg Descriptor Registry.json'),work/'fmg-registry.json')
    registry=json.loads(registry_path.read_text())
    source_hash=hashlib.sha256(HELPER_SOURCE.encode()).hexdigest()
    helper=work/'helper'; helper.mkdir(exist_ok=True)
    project=helper/'Messages.csproj'
    refs=[]
    for name in ['SoulsFormats.dll','BouncyCastle.Cryptography.dll','DrSwizzler.dll','ZstdNet.dll']:
        path=library.parent/name
        if path.is_file():refs.append(f'<Reference Include="{html.escape(path.stem)}"><HintPath>{html.escape(str(path))}</HintPath><Private>true</Private></Reference>')
    project_source='<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net9.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings><Nullable>enable</Nullable></PropertyGroup><ItemGroup>'+''.join(refs)+'</ItemGroup></Project>'
    stamp=helper/'source.sha256';assembly=helper/'bin/Release/net9.0/Messages.dll'
    if not stamp.is_file() or stamp.read_text()!=source_hash or not assembly.is_file():
        project.write_text(project_source);(helper/'Program.cs').write_text(HELPER_SOURCE)
        env=os.environ.copy();env['DOTNET_CLI_HOME']=str(work/'dotnet-home');env['NUGET_PACKAGES']=str(work/'nuget');env['DOTNET_CLI_TELEMETRY_OPTOUT']='1'
        with (work/'helper-build.log').open('w') as log:
            subprocess.run([str(dotnet),'build',str(project),'-c','Release','--nologo'],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        stamp.write_text(source_hash)
    return registry,keys,assembly


def labels(ids, polish=False):
    roman=['I','II','III','IV','V','VI','VII','VIII','IX','X','XI','XII','XIII','XIV','XV','XVI']
    result=[]
    for index,identifier in enumerate(ids):
        if index<16:
            if polish:
                title='Odłamek Pieczęci '+roman[index]
                summary='Cząstka dawnej pieczęci, związana ze swym nosicielem.'
                description='Odłamek pieczęci, której znaku nie nosi żaden ród.\n\nPrzechowuje cichy ślad pokonanych prób. Jego blask odpowiada tylko temu, kto go niesie.'
            else:
                title='Seal Fragment '+roman[index]
                summary='A fragment of an ancient seal, bound to its bearer.'
                description='A fragment of a seal bearing no lord\'s crest.\n\nIt holds a quiet memory of trials overcome. Its pale light answers only to the one who carries it.'
        else:
            if polish:
                title='Pieczęć Wzniesienia';summary='Dawna pieczęć, która pamięta drogę swego nosiciela.'
                description='Pieczęć złożona z szesnastu milczących odłamków.\n\nNie przyrzeka łaski żadnego bóstwa. Pamięta jedynie trudy Zmatowieńca i moc, którą zdołał sobie wywalczyć.'
            else:
                title='Ascension Seal';summary='An ancient seal that remembers its bearer\'s path.'
                description='A seal joined from sixteen silent fragments.\n\nIt promises no god\'s favor. It remembers only a Tarnished\'s trials, and the strength they earned beyond them.'
        result.append({'id':identifier,'title':title,'summary':summary,'description':description})
    return result


def windows_path(path):
    return 'Z:' + str(Path(path).resolve()).replace('/', '\\')


def prepare_runner(work, game, assembly):
    """Use the owned Windows Oodle through isolated tooling Wine, never the game prefix."""
    wine=shutil.which('wine')
    if not wine:raise ValueError('Wine is required to use the installed Windows Oodle library for native message compression')
    archive=fetch(WINDOWS_RUNTIME_URL,work/f'dotnet-runtime-{WINDOWS_RUNTIME}-win-x64.zip')
    if hashlib.sha512(archive.read_bytes()).hexdigest()!=WINDOWS_RUNTIME_SHA512:
        raise ValueError('Official Windows .NET runtime SHA512 differs from pinned release metadata')
    runtime=work/f'dotnet-runtime-{WINDOWS_RUNTIME}-win-x64'
    if not (runtime/'dotnet.exe').is_file():extract_archive(archive,runtime)
    original_oodle=game/'oo2core_6_win64.dll'
    if not original_oodle.is_file():raise ValueError('Installed game Oodle library is missing')
    # The private tooling copy is not a deliverable and never enters mod/ or dist/.
    oodle=work/'oo2core_6_win64.dll'
    if not oodle.is_file() or digest(oodle)!=digest(original_oodle):shutil.copyfile(original_oodle,oodle)
    env=os.environ.copy()
    env.update({'WINEPREFIX':str(work/'wineprefix'),'WINEARCH':'win64','WINEDEBUG':'-all',
                'WINEDLLOVERRIDES':'winemenubuilder.exe=d','DISPLAY':'',
                'ASCENSION_OODLE':windows_path(oodle),'DOTNET_CLI_TELEMETRY_OPTOUT':'1',
                'DOTNET_CLI_HOME':windows_path(work/'dotnet-home')})
    command=[wine,str(runtime/'dotnet.exe'),windows_path(assembly)]
    return command,env,{'version':WINDOWS_RUNTIME,'url':WINDOWS_RUNTIME_URL,'sha512':WINDOWS_RUNTIME_SHA512,
                        'oodle_source_sha256':digest(original_oodle)}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dotnet',type=Path,default=Path(shutil.which('dotnet') or DEFAULT_DOTNET))
    parser.add_argument('--soulsformats',type=Path,default=DEFAULT_LIBRARY)
    parser.add_argument('--game-dir',type=Path)
    parser.add_argument('--work-root',type=Path,default=WORKSPACE/'work/tooling/messages')
    parser.add_argument('--extract-only',action='store_true',help='Only generate the selectively extracted local source cache')
    args=parser.parse_args()
    try:
        _lib,detected,_state,_manifest=locate(steam_root(),args.game_dir)
        game=detected.resolve();exe=game/'eldenring.exe'
        compatibility=json.loads((PROJECT/'config/compatibility.json').read_text())
        if digest(exe) not in compatibility['sha256'] or pe_version(exe)['product_version']!=compatibility['product_version']:
            raise ValueError('Unsupported executable fingerprint')
        registry,keys,assembly=prepare(args.work_root.resolve(),args.dotnet.resolve(),args.soulsformats.resolve())
        containers=[c for c in registry['Containers'] if c['ContainerName']=='Item' and not c.get('Obsolete',False)]
        if len(containers)!=1:raise ValueError('Cannot select a unique active native item text binder')
        container=containers[0]['FullName'];languages=[row['Abbreviation'] for row in registry['Languages']]
        source_key=digest(game/'Data0.bhd')
        root=WORKSPACE/'work/messages';source_root=root/'sources'/source_key
        source_root.mkdir(parents=True,exist_ok=True)
        runner,env,runtime_provenance=prepare_runner(args.work_root.resolve(),game,assembly)
        cache_manifest=source_root/'source-manifest.json'
        if not cache_manifest.is_file():
            with (source_root/'extraction.log').open('w') as log:
                subprocess.run([*runner,'extract',windows_path(game),windows_path(keys),windows_path(source_root),','.join(languages),container],env=env,cwd=assembly.parent,stdout=log,stderr=subprocess.STDOUT,check=True)
        sources=json.loads(cache_manifest.read_text())
        for archive in sources['archives']:
            if digest(game/archive['name'])!=archive['sha256']:raise ValueError('Source archive headers changed; remove only the old local source cache and regenerate')
        available={row['path'].split('/')[1]:row for row in sources['files']}
        if not {'engus','polpl'}.issubset(available):raise ValueError('English and Polish active item binders were not found in installed archives')
        if args.extract_only:
            print(json.dumps({'source_root':str(source_root),'languages':sorted(available),'container':container},indent=2));return 0
        marker=json.loads((PROJECT/'config/identity-marker.json').read_text());ids=marker['payload_goods']+[marker['commit_good']]
        if len(ids)!=17 or len(set(ids))!=17:raise ValueError('Identity marker must declare exactly17distinct Goods IDs')
        roles={str(row['ID']):{'Title':'title','Summary':'summary','Description':'description'}[row['Role']]
               for row in registry['List'] if row.get('Container')=='Item' and row.get('Domain')=='Goods' and row.get('Role') in {'Title','Summary','Description'}}
        build=root/('build-'+uuid.uuid4().hex[:12]);build.mkdir()
        generated=[]
        for language,source in sorted(available.items()):
            relative=source['path'];original=source_root/relative
            if digest(original)!=source['source_sha256']:raise ValueError('Cached source binder hash differs')
            entries=labels(ids,language=='polpl')
            label_file=build/(language+'-labels.json');label_file.write_text(json.dumps({'roles':roles,'entries':entries},ensure_ascii=False,indent=2))
            merged=build/relative
            with (build/(language+'-merge.log')).open('w') as log:
                subprocess.run([*runner,'merge',windows_path(original),windows_path(merged),windows_path(label_file)],env=env,cwd=assembly.parent,stdout=log,stderr=subprocess.STDOUT,check=True)
            output=PROJECT/'mod'/relative
            for path in [PROJECT/'mod',PROJECT/'mod/msg',output.parent]:
                if path.is_symlink():raise ValueError('Message output parent cannot be a symlink')
            output.parent.mkdir(parents=True,exist_ok=True)
            if output.is_symlink():raise ValueError('Message output cannot be a symlink')
            temporary=output.with_name(output.name+'.new-'+uuid.uuid4().hex)
            with merged.open('rb') as src,temporary.open('xb') as dst:
                shutil.copyfileobj(src,dst);dst.flush();os.fsync(dst.fileno())
            os.replace(temporary,output)
            generated.append({'path':relative,'sha256':digest(output),'source_sha256':source['source_sha256'],
                              'source_archive':source['archive'],'language':language,'localization':'polish' if language=='polpl' else 'english' if language=='engus' else 'english_fallback',
                              'tables':json.loads(Path(str(merged)+'.merge.json').read_text())['tables']})
        result={'schema':1,'source_executable_sha256':digest(exe),'active_container':container,
                'goods_ids':ids,'generated_files':generated,'source_archive_headers':sources['archives'],
                'source_library':{'repository':'soulsmods/SoulsFormatsNEXT','revision':SOURCE_REV,'assembly_sha256':digest(args.soulsformats)},
                'schema_registry_revision':SMITHBOX_REV,'tooling_runtime':runtime_provenance,
                'status':'generated; no tests, round-trip or game launch performed'}
        text=json.dumps(result,indent=2,ensure_ascii=False)+'\n'
        (build/'messages-manifest.json').write_text(text)
        target=PROJECT/'config/messages-manifest.json';tmp=target.with_name(target.name+'.new-'+uuid.uuid4().hex);tmp.write_text(text);os.replace(tmp,target)
        print(json.dumps({'files':len(generated),'languages':sorted(available),'container':container,'goods':17,'build':str(build),'manifest':str(target),'status':result['status']},indent=2))
        return 0
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as error:
        print('MESSAGE BUILD ERROR: '+str(error),file=sys.stderr);return 2

if __name__=='__main__':raise SystemExit(main())
