#!/usr/bin/env bash
python3 -c "
import zipfile, os, xml.etree.ElementTree as ET

total = 0
has_rating = 0
no_rating = 0
no_comicinfo = 0

for root, dirs, files in os.walk('/mnt/user/Downloads/Finished/Comics'):
    for f in files:
        if not f.lower().endswith(('.cbz','.cbr')):
            continue
        total += 1
        path = os.path.join(root, f)
        try:
            z = zipfile.ZipFile(path)
            if 'ComicInfo.xml' not in z.namelist():
                no_comicinfo += 1
                continue
            xml = z.read('ComicInfo.xml').decode('utf-8')
            tree = ET.fromstring(xml)
            age = tree.findtext('AgeRating')
            if age and age.lower() not in ('', 'unknown', 'rating pending'):
                has_rating += 1
            else:
                no_rating += 1
        except:
            no_rating += 1

print(f'Total files:        {total}')
print(f'Has age rating:     {has_rating} ({100*has_rating//max(total,1)}%)')
print(f'No/unknown rating:  {no_rating}')
print(f'No ComicInfo.xml:   {no_comicinfo}')
"