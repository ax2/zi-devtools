import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
from package_view_operation_revision import digest, validate_revision, validate_proof


class ViewNavigationRevisionTests(unittest.TestCase):
    def fixtures(self):
        for name in ['agent-assist-preview','spring-actuator']:
            baseline = ROOT/'release/plugin-candidates'/(name+'-0.1.2-candidate-01')
            source = ROOT/'plugins'/name
            yield [json.loads((base/file).read_bytes()) for base,file in [
                (baseline,'plugin.json'),(source,'plugin.json'),(baseline,'catalog.json'),(source,'catalog.json')]]

    def test_exact_mapping_only(self):
        for values in self.fixtures():
            validate_revision(*values)

    def test_rejects_executable_or_permission_drift(self):
        for values in self.fixtures():
            for field in ['permissions','capability','profile','operation','version','sourceScope']:
                v=copy.deepcopy(values)
                if field=='permissions':v[1]['permissions']={}
                elif field=='capability':v[1]['contributes']['capabilities']=[{'id':'fake'}]
                elif field=='profile':v[1]['requiredProfiles']['view.operation.directory.v1']='*'
                elif field=='operation':v[1]['contributes']['viewOperations'][0]['runtime']='fake'
                elif field=='version':v[1]['version']='0.2.0'
                else:v[3]['sourceScopeComplete']=True
                with self.assertRaises(ValueError,msg=field):validate_revision(*v)

    def proof_fixture(self):
        source=ROOT/'plugins/agent-assist-preview'
        data=(source/'plugin.json').read_bytes();manifest=json.loads(data);view=(source/'views/main.html').read_bytes()
        proof={'status':'passed','runtimeAccepted':False,'cases':[
            {'operationId':op['id'],'theme':theme,'viewSha256':digest(view),'manifestSha256':digest(data),
             'status':'opened','busyStatus':'unsupported','noAutomaticRequest':True,'preservesDraftAndResult':True}
            for op in manifest['contributes']['viewOperations'] for theme in ['light','dark']]}
        return proof,manifest,view,data

    def test_proof_binding(self):
        validate_proof(*self.proof_fixture())

    def test_rejects_missing_duplicate_stale_or_failed_proof(self):
        for scenario in ['missing','duplicate','stale','request','draft','busy','hostAccepted']:
            proof,manifest,view,data=self.proof_fixture()
            if scenario=='missing':proof['cases'].pop()
            elif scenario=='duplicate':proof['cases'][0]=proof['cases'][1]
            elif scenario=='stale':view+=b' '
            elif scenario=='request':proof['cases'][0]['noAutomaticRequest']=False
            elif scenario=='draft':proof['cases'][0]['preservesDraftAndResult']=False
            elif scenario=='busy':proof['cases'][0]['busyStatus']='opened'
            else:proof['runtimeAccepted']=True
            with self.assertRaises(ValueError,msg=scenario):validate_proof(proof,manifest,view,data)


if __name__=='__main__':unittest.main()
