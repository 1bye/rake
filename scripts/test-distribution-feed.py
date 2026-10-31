#!/usr/bin/env python3
"""Actual verified TLS and byte-range contract required by managed MSIX downloads."""
import pathlib
import ssl
import tempfile
import urllib.error
import urllib.request
from distribution_test_feed import TestFeed

with tempfile.TemporaryDirectory(prefix='rake-feed-contract-') as directory:
    with TestFeed(pathlib.Path(directory)) as feed:
        (feed.site / 'package.msix').write_bytes(b'0123456789')
        context = ssl.create_default_context(cafile=str(feed.ca))
        def request(method='GET', value=None):
            headers = {'Range': value} if value else {}
            return urllib.request.urlopen(urllib.request.Request(feed.base_url + '/package.msix', headers=headers, method=method), context=context)
        with request('HEAD') as response:
            assert response.status == 200 and response.headers['Content-Length'] == '10'
            assert response.headers['Accept-Ranges'] == 'bytes'
            assert response.headers['Content-Type'] == 'application/msix' and not response.read()
        for value, expected, header in [('bytes=2-4', b'234', 'bytes 2-4/10'), ('bytes=7-', b'789', 'bytes 7-9/10'), ('bytes=-2', b'89', 'bytes 8-9/10'), ('bytes=8-99', b'89', 'bytes 8-9/10')]:
            with request(value=value) as response:
                assert response.status == 206 and response.read() == expected
                assert response.headers['Content-Range'] == header
                assert int(response.headers['Content-Length']) == len(expected)
        for value in ['bytes=99-', 'bytes=8-2', 'bytes=-0', 'bytes=0-1,3-4', 'bytes=-']:
            try:
                request(value=value)
                raise AssertionError('invalid range accepted')
            except urllib.error.HTTPError as error:
                assert error.code == 416 and error.headers['Content-Range'] == 'bytes */10'
                assert not error.read()
        with request() as response:
            assert response.status == 200 and response.read() == b'0123456789'
print('Verified HTTPS HEAD/MIME, partial/open/suffix ranges, framing and unsatisfiable ranges passed')
