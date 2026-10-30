from pathlib import Path
import argparse, io, re, subprocess, tarfile
parser=argparse.ArgumentParser(description="Check Linux release package metadata, resources and checksums")
parser.add_argument("--directory",type=Path,required=True)
parser.add_argument("--version",required=True)
args=parser.parse_args()
assert re.fullmatch(r"\d+\.\d+\.\d+",args.version),"invalid release version"
root=args.directory.resolve()
checks=root/'SHA256SUMS-linux.txt'
subprocess.run(['sha256sum','--check',checks.name],cwd=root,check=True)
packages={suffix:list(root.glob('*'+suffix)) for suffix in ['.AppImage','.deb','.rpm']}
for suffix,files in packages.items():
    assert len(files)==1,(suffix,files)
    print(files[0].name,files[0].stat().st_size,'bytes')
image=packages['.AppImage'][0]
with image.open('rb') as f: assert f.read(4)==b'\x7fELF','AppImage is not an ELF executable'
data=packages['.deb'][0].read_bytes();assert data.startswith(b'!<arch>\n')
pos=8;members={}
while pos+60<=len(data):
    header=data[pos:pos+60];size=int(header[48:58]);name=header[:16].decode().strip().rstrip('/')
    members[name]=data[pos+60:pos+60+size];pos+=60+size+(size%2)
control=next(v for k,v in members.items() if k.startswith('control.tar'))
with tarfile.open(fileobj=io.BytesIO(control)) as tar:
    member=next(m for m in tar if m.name.lstrip('./')=='control')
    info=tar.extractfile(member).read().decode();print(info)
    assert re.search(r'^Version: '+re.escape(args.version)+r'$',info,re.M)
    assert re.search(r'^Architecture: amd64$',info,re.M)
payload=next(v for k,v in members.items() if k.startswith('data.tar'))
with tarfile.open(fileobj=io.BytesIO(payload)) as tar:
    entries=list(tar)
    for tool in ['yt-dlp','ffmpeg','ffprobe']:
        found=[m for m in entries if m.name.endswith('/tools/'+tool)]
        assert len(found)==1 and found[0].mode&0o111,(tool,[(m.name,oct(m.mode)) for m in found])
        print('Executable tool:',found[0].name,oct(found[0].mode))
    for name in ['FFmpeg-LICENSE.txt','FFmpeg-SOURCE.txt','yt-dlp-LICENSE.txt','Rabbit-LICENSE.txt','THIRD_PARTY_NOTICES.md']:
        assert any(m.name.endswith('/'+name) for m in entries),name
print('Package checksums, Debian metadata, private tools and license resources verified.')

rpm=packages['.rpm'][0]
rpm_version=subprocess.check_output(['rpm','-qp','--qf','%{VERSION}',str(rpm)],text=True).strip()
assert rpm_version==args.version,(rpm_version,args.version)
rows=subprocess.check_output(['rpm','-qp','--qf','[%{FILENAMES}\t%{FILEMODES:perms}\n]',str(rpm)],text=True).splitlines()
for tool in ['yt-dlp','ffmpeg','ffprobe']:
    found=[line.split('\t') for line in rows if line.split('\t')[0].endswith('/tools/'+tool)]
    assert len(found)==1 and 'x' in found[0][1],(tool,found)
    print('RPM executable tool:',*found[0])
print('RPM version and executable tool permissions verified.')
