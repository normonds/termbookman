# ls
ls -lpah --color | awk '{k=0;for(i=0;i<=8;i++)k+=((substr($1,i+2,1)~/[rwx]/)*2^(8-i));if(k)printf("%0o ",k);print}'
cat /proc/cpuinfo
# info linux
printf 'whoami: ';whoami;printf 'uname -a: ';uname -a;lsb_release -a;lscpu | grep "CPU(s):\\|Model name\\|per socket\\|CPU MHz\\|Vendor ID";awk '$3=="kB"{if ($2>1024^2){$2=$2/1024^2;$3="GB";} else if ($2>1024){$2=$2/1024;$3="MB";}} 1' /proc/meminfo | column -t | grep "MemTotal\\|MemFree\\|SwapTotal\\|SwapFree"

# add favourite to ubuntu taskbar
gsettings set org.gnome.shell favorite-apps "$(gsettings get org.gnome.shell favorite-apps | sed s/.$//), '<prompt:desktop file in /usr/share/applications:*.desktop>.desktop']"
# ps linux processes
ps -afx
# top linux processes
echo "press V tx2 mx2";top -c -E g
# release linux info
lsb_release -a

gcloud components update
gcloud config list && echo;echo;gcloud app describe
gcloud projects list

ibmcloud update; ibmcloud plugin update
# make active nvm node sudoable
n=$(which node); n=${n%/bin/node}; chmod -R 755 $n/bin/*; sudo cp -r $n/{bin,lib,share} /usr/local
# kubernetes ibmcloud info
echo --- NODES ---;kubectl get nodes;echo --- PODS ---;kubectl get pods;echo --- SERVICES ---; kubectl get services
# kubernetes ibmcloud workers info
ibmcloud ks workers --cluster <prompt:cluster id:bku5rfnf0tefg7alcca0>

# kubernetes conn terminal
pods=$(kubectl get pods -o name) && pod=$(eval echo $pods | cut -d'/' -f 2) && echo kubectl exec -it $pod -- top && eval kubectl exec -it $pod -- byobu
# l linux filesystem list al files in directory
ls -ahsXp --color
# ls linux filesystem list files permissions
ls -lpah --color | awk '{k=0;for(i=0;i<=8;i++)k+=((substr($1,i+2,1)~/[rwx]/)*2^(8-i));if(k)printf("%0o ",k);print}'
# volume linux format sudo
mkfs -t ext4 /dev/someVolume

# disks filesystem size disks sudo
fdisk -l
# disks2 filesystem size disks
df -h
# disks3 filesystem size
lsblk -o NAME,FSTYPE,LABEL,SIZE,UUID,MOUNTPOINT
# size filesystem files in current directory
find -type f | wc -l
# size2 filesystem current directory size
du -hs <prompt:directory:.>
# size3 ncdu app show largest files
ncdu -x <prompt:directory:/>
# ports tcp udp
netstat --tcp --udp --listening --programs --numeric | grep -i "<prompt:grep:>"  --color=always 
# ports2 opened ports sudo
lsof -i -P -n | grep LISTEN --color=always
# system info
lshw -short
# cpu system info
lscpu
# directory current
pwd
# iptables linux firewall show  sudo
iptables -L
# crontab linux edit sudo (@reboot /path/to/script.sh)
crontab -e
# iptablesaccept linux firewall accept all connections for session sudo
iptables -P INPUT ACCEPT && iptables -P OUTPUT ACCEPT && iptables -P FORWARD ACCEPT && iptables -F
# linux systemctl service status
systemctl status <prompt:service name:cron>
# chmod info
echo "7:rwx,6:rw-,5:r-x,4:r--,3:-wx,2:-w-,1:--x,0---"

#imagemagick resize pngs to out folder
mogrify -monitor -path out -resize 33% -format png *.png
#imagemagick convert jpgs to pngs imgs
mogrify -format png *.jpg
# ip info url request
curl https://ifconfig.co
# ipjson info url request
curl https://ifconfig.co/json
# port info url request
curl https://ifconfig.co/port/<prompt:port:8080>
# nodejs lts install sudo
curl -sL https://deb.nodesource.com/setup_<prompt:LTS version 10,12,14:14>.x | sudo -E bash - && sudo apt-get install nodejs
# grep linux search string in files nonbinary recursive caseinsensitive
grep -rni<prompt:ignore binary files?:I:> --include <prompt:filetype:*.*> "<prompt:search string>" <prompt:directory:.> <prompt:use less? (else fzf):--color=always | less -r: | fzf>
# find linux search file
find <prompt:directory:/home> -iname "*<prompt:string in filenamepath:json>*" -print 2>/dev/null | grep <prompt:string in filenamepath:json> -i <prompt:use less? (else fzf):--color=always | less -r: | fzf>
# findtext
ack "<prompt:string in file:"
# append to file
echo "<prompt:string>" >> <prompt:filename:file.txt>
# replace text in file
sed -i "s/<prompt:search string>/<prompt:replace string>/g" <prompt:filename:file.txt>
# head  linux show first lines of file
head -<prompt:first n lines:30> <prompt:file:file.txt>

mysqldump --user=<prompt:user:root> --password=<prompt:password:1234> <prompt:database> --result-file <prompt:file to save>
# zipnew linux
zip <prompt:zipped file:file.zip> <prompt:file to zip:file.txt>

# gitdiff git latest difference
git show
# gitpull
git fetch --all && git pull
# gitlog
git log --graph --decorate --pretty=oneline --abbrev-commit
# gitpush publish
git push -u origin master
# gitcommit git add commit publish
git add <prompt:directory:.> && git commit -m "<prompt:message:->" && git push <prompt:remote:origin> <prompt:branch:master>
# gitreset replace curr dir files with latest remote commit
git pull origin <prompt:branch:master> && git reset --hard HEAD
# gitsquashbranch git squash branch
git checkout --orphan new-master <prompt:branch:master> && git commit -m "<prompt:message:initial commit>" && git branch -M new-master <prompt:branch:master>
#git remote
git remote -v

#docksystem docker system usage info
sudo docker system df -v
# dockprune docker delete all containers
sudo docker system prune --all --force
#dockbash in container
sudo docker exec -t -i <prompt:container name:node> /bin/bash
# docktop docker stats
sudo docker stats
# dockstopall docker containers sudo
sudo docker stop $(sudo docker ps -a -q)
# dockerlogs
sudo docker logs --follow <prompt:container:>
# dockcont docker list containers
sudo docker ps -as

# npmglobal list packages
npm list -g --depth 0
# npmoutdated glob packages
npm outdated -g --depth=0

# package info
apt-cache show <prompt:package name:google-chrome-stable>

# youtube 1 fps
ffmpeg -loop 1 -framerate 1 -i <prompt:image file> -i <prompt:mp3 file> -c:v libx264 -preset veryslow -crf 0 -c:a copy -shortest <prompt:output file:output.mp4>

# editor open pipe
apt-cache show nano | nano -
# editor
echo "nano: F1 - help;SHIFT+ALT+4 - toogle word wrap; ALT+U - undo; ALT+N - redo;CTRL+^ - start text mark; CTRL+K - cuts selected text;CTRL+U - paste;F6 - search; ALT+W - repeat search;ALT+C - toogle info box"
# fzf file browser
fzf --preview='head -30 {}'
# launcher create desktop shortcut
printf "[Desktop Entry]\nName=<prompt:App name:gEdit>\nExec=<prompt:path:/usr/bin/gedit>\nIcon=<prompt:icon:/usr/share/icons/Humanity/apps/48/gedit-icon.svg>\nTerminal=<prompt:terminal:false>\nType=Application" > <prompt:shortcut folder:~/Desktop>/<prompt:shortcut name:gEdit>.desktop; chmod +x <prompt:shortcut folder:~/Desktop>/<prompt:shortcut name:gEdit>.desktop
# ubuntu 20.04 desktop icons
sudo -H gedit /usr/share/gnome-shell/extensions/desktop-icons@csoriano/prefs.js
# wifi devices
iw dev
# fail2ban status
sudo fail2ban-client status sshd
# gsutil list buckets
gsutil ls