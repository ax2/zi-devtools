import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'scripts'))
from package_application_directory_revision import BASELINES, validate_revision
from tool_directory import validate_directory


class ApplicationDirectoryTests(unittest.TestCase):
    def fixtures(self):
        for name, (baseline, _, candidate) in BASELINES.items():
            yield [json.loads((ROOT/path/file).read_bytes()) for path,file in [
                (baseline,'plugin.json'),('release/plugin-candidates/'+candidate,'plugin.json'),
                (baseline,'catalog.json'),('release/plugin-candidates/'+candidate,'catalog.json')]]

    def test_only_application_metadata_and_version_change(self):
        for values in self.fixtures():
            validate_revision(*values)

    def test_rejects_unsupported_profile_and_fake_capability(self):
        for values in self.fixtures():
            for field in ['profile','capability','view','permissions']:
                changed=copy.deepcopy(values)
                if field=='profile':
                    changed[1]['requiredProfiles']['view.operation.directory.v1']='=1.0.0-preview.1'
                elif field=='capability':
                    changed[1]['contributes']['capabilities']=[{'id':'fake.execute'}]
                elif field=='view':
                    changed[1]['contributes']['views'][0]['entry']='other.html'
                else:
                    changed[1]['permissions']={}
                with self.assertRaises(ValueError):
                    validate_revision(*changed)

    def test_rejects_source_version_drift(self):
        for values in self.fixtures():
            values[3]['unexpectedScope']='complete'
            with self.assertRaises(ValueError):
                validate_revision(*values)

    def test_alias_normalization_size_and_control_limits(self):
        for aliases in [['AI','ＡＩ'],['a',' A '],['bad\n'],['中'*43],['x']*17]:
            with self.assertRaises(AssertionError):
                validate_directory({'categoryId':'ai','aliases':aliases})

    def test_proposal_vectors_cover_six_operations_without_claiming_runtime(self):
        folder=ROOT/'proposals/studio-view-operation-directory/2026-10-18'
        proposal=json.loads((folder/'operations.json').read_bytes())
        vectors=json.loads((folder/'vectors.json').read_bytes())
        self.assertFalse(proposal['profileDeclared'])
        self.assertEqual(vectors['status'],'specification_only_not_runtime_evidence')
        operations=proposal['operations']
        self.assertEqual(len(operations),6)
        ids={op['id'] for op in operations}
        self.assertEqual(len(ids),6)
        for op in operations:
            validate_directory(op['directory'])
            self.assertFalse(op['autoExecute'])
            name='agent-assist-preview' if op['sourceToolId']=='agent-runner' else 'spring-actuator'
            manifest=json.loads((ROOT/'plugins'/name/'plugin.json').read_bytes())
            self.assertEqual(manifest['id'],op['packageId'])
            self.assertIn(op['viewId'],[v['id'] for v in manifest['contributes']['views']])
            rows=[v for v in vectors['vectors'] if v['declaredOperationId']==op['id']]
            self.assertEqual(len(rows),5)
            self.assertEqual({v['scenario'] for v in rows},
                             {'located','unknown','slow_callback','disabled','version_changed'})
            for row in rows:
                self.assertEqual(row['expected']['modelCalls'],0)
                self.assertEqual(row['expected']['httpCalls'],0)
                self.assertFalse(row['expected']['permissionsGranted'])
                self.assertFalse(row['expected']['draftChanged'])
                if row['scenario'] in ('slow_callback','disabled','version_changed'):
                    self.assertTrue(row['expected']['lateReplyIgnored'])
                    self.assertTrue(row['expected']['lateFocusForbidden'])
        self.assertEqual(len(vectors['vectors']),30)


if __name__ == '__main__':
    unittest.main()
